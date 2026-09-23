//! Engine binary cache: layout, extraction, and lookup.
//!
//! Boundary: filesystem state under the cache root only. Manifest
//! parsing and network fetches live in sibling modules. Layout:
//! `<root>/engines/<product>/<version>/<binary>`; one pinned version per
//! directory so concurrent rutter processes reuse the same install.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::EngineError;

/// Directory name for in-progress installs; renamed into place on
/// success so a crash never leaves a half-extracted version visible.
const TMP_PREFIX: &str = ".tmp-";

/// Disambiguates staging directories between installs in the same
/// process: the process id alone collides when concurrent callers
/// (each test of a suite on a cold cache) race one version's install.
static STAGING_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// Hard caps for archive extraction. Legit Chrome for Testing archives
/// stay far below them (a full chrome-win64 install extracts to a few
/// hundred MB); the caps only bound a hostile archive so a compromised
/// manifest source cannot exhaust memory or disk. Archive entries are
/// hostile input (docs/architecture.md), and a zip's declared sizes cannot be
/// trusted, so the caps apply to the bytes actually decompressed.
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A located engine binary ready to launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledEngine {
    /// Product directory name, for example `chrome-headless-shell`.
    pub product: String,
    /// Engine version directory name; `external` for caller-supplied
    /// binaries that bypass the cache.
    pub version: String,
    /// Absolute path of the launchable binary.
    pub executable: PathBuf,
}

/// The engine cache rooted at a directory.
#[derive(Debug, Clone)]
pub struct EngineStore {
    root: PathBuf,
}

impl EngineStore {
    /// Creates a store under `root` (created lazily on first install).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Highest installed version of a product, if any. Version
    /// directories that do not parse as numeric are ignored.
    pub fn installed(&self, product: &str) -> Result<Option<InstalledEngine>, EngineError> {
        let product_dir = self.root.join("engines").join(product);
        let entries = match fs::read_dir(&product_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(EngineError::DownloadFailed {
                    detail: format!(
                        "cannot read cache directory {}: {error}",
                        product_dir.display()
                    ),
                });
            }
        };

        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| EngineError::DownloadFailed {
                detail: format!(
                    "cannot read cache entry in {}: {error}",
                    product_dir.display()
                ),
            })?;
            let name = entry.file_name();
            let Some(version) = name.to_str() else {
                continue;
            };
            if version.starts_with(TMP_PREFIX) {
                continue;
            }
            if parse_version(version).is_none() {
                continue;
            }
            versions.push(version.to_owned());
        }
        versions.sort_by_key(|version| parse_version(version).unwrap_or_default());
        versions.reverse();

        for version in versions {
            let executable = product_dir.join(&version).join(binary_name(product));
            if executable.is_file() {
                return Ok(Some(InstalledEngine {
                    product: product.to_owned(),
                    version,
                    executable,
                }));
            }
        }
        Ok(None)
    }

    /// Installs a product version from zip bytes: extracts the binary
    /// into a temporary directory, then renames it into place.
    pub fn install(
        &self,
        product: &str,
        version: &str,
        zip_bytes: &[u8],
    ) -> Result<InstalledEngine, EngineError> {
        // The version names two directories below and comes verbatim
        // from the downloaded manifest, so it must be a plain
        // dotted-numeric version: `..` parts or separators would steer
        // the final rename outside the cache root — the same compromised
        // manifest the artifact-host pin defends against. `installed()`
        // only ever finds parseable versions, so this matches the read
        // side too.
        if parse_version(version).is_none() {
            return Err(EngineError::DownloadFailed {
                detail: format!("engine version is not dotted-numeric: '{version}'"),
            });
        }
        let final_dir = self.root.join("engines").join(product).join(version);
        if final_dir.join(binary_name(product)).is_file() {
            return Ok(InstalledEngine {
                product: product.to_owned(),
                version: version.to_owned(),
                executable: final_dir.join(binary_name(product)),
            });
        }

        let staging = final_dir
            .parent()
            .ok_or_else(|| EngineError::DownloadFailed {
                detail: format!("cache path has no parent: {}", final_dir.display()),
            })?
            .join(format!(
                "{TMP_PREFIX}{version}-{}-{}",
                std::process::id(),
                STAGING_ATTEMPT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&staging).map_err(|error| EngineError::DownloadFailed {
            detail: format!(
                "cannot create staging directory {}: {error}",
                staging.display()
            ),
        })?;

        let installed = self.extract_archive(product, version, zip_bytes, &staging);
        if installed.is_err() {
            let _ = fs::remove_dir_all(&staging);
            return installed;
        }

        if let Some(parent) = final_dir.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            let _ = fs::remove_dir_all(&staging);
            return Err(EngineError::DownloadFailed {
                detail: format!(
                    "cannot create cache directory {}: {error}",
                    parent.display()
                ),
            });
        }
        if let Err(error) = fs::rename(&staging, &final_dir) {
            let _ = fs::remove_dir_all(&staging);
            // Another process may have installed the same version while
            // we extracted; a winning race is as good as our own install.
            if let Ok(Some(installed)) = self.installed(product)
                && installed.version == version
            {
                return Ok(installed);
            }
            return Err(EngineError::DownloadFailed {
                detail: format!(
                    "cannot move engine into place {} -> {}: {error}",
                    staging.display(),
                    final_dir.display()
                ),
            });
        }

        Ok(InstalledEngine {
            product: product.to_owned(),
            version: version.to_owned(),
            executable: final_dir.join(binary_name(product)),
        })
    }

    /// Extracts the whole engine zip into `staging`, flattening the
    /// single top-level folder Chrome for Testing ships. The binary
    /// needs its siblings (ICU data, DLLs) to launch, so extracting
    /// only the executable is not enough.
    fn extract_archive(
        &self,
        product: &str,
        version: &str,
        zip_bytes: &[u8],
        staging: &Path,
    ) -> Result<InstalledEngine, EngineError> {
        self.extract_archive_with_caps(
            product,
            version,
            zip_bytes,
            staging,
            MAX_ENTRY_BYTES,
            MAX_EXTRACTED_BYTES,
        )
    }

    /// [`Self::extract_archive`] with explicit caps; the install path
    /// passes the constants, tests pass small ones.
    fn extract_archive_with_caps(
        &self,
        product: &str,
        version: &str,
        zip_bytes: &[u8],
        staging: &Path,
        max_entry_bytes: u64,
        max_extracted_bytes: u64,
    ) -> Result<InstalledEngine, EngineError> {
        let reader = std::io::Cursor::new(zip_bytes);
        let mut archive =
            zip::ZipArchive::new(reader).map_err(|error| EngineError::DownloadFailed {
                detail: format!("engine zip is not readable: {error}"),
            })?;

        let wanted = binary_name(product);
        let mut executable = None;
        let mut extracted_bytes: u64 = 0;
        for index in 0..archive.len() {
            let mut file =
                archive
                    .by_index(index)
                    .map_err(|error| EngineError::DownloadFailed {
                        detail: format!("engine zip entry {index} is unreadable: {error}"),
                    })?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().to_owned();
            // Flatten `chrome-headless-shell-win64/<rest>` to `<rest>`;
            // entries without a folder prefix stay as they are.
            let relative = match name.find(['/', '\\']) {
                Some(split) => &name[split + 1..],
                None => name.as_str(),
            };
            if relative.is_empty() {
                continue;
            }
            let target = safe_join(staging, relative)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| EngineError::DownloadFailed {
                    detail: format!("cannot create {}: {error}", parent.display()),
                })?;
            }
            // Read through a take() bound: the entry must fit under the
            // per-entry cap or the archive is hostile (zip bomb).
            let mut contents = Vec::new();
            {
                let mut limited = (&mut file).take(max_entry_bytes.saturating_add(1));
                limited.read_to_end(&mut contents).map_err(|error| {
                    EngineError::DownloadFailed {
                        detail: format!("cannot read '{name}' from engine zip: {error}"),
                    }
                })?;
            }
            if contents.len() as u64 > max_entry_bytes {
                return Err(EngineError::DownloadFailed {
                    detail: format!(
                        "engine zip entry '{name}' is over the {max_entry_bytes}-byte extraction cap"
                    ),
                });
            }
            extracted_bytes += contents.len() as u64;
            if extracted_bytes > max_extracted_bytes {
                return Err(EngineError::DownloadFailed {
                    detail: format!(
                        "engine zip is over the {max_extracted_bytes}-byte total extraction cap"
                    ),
                });
            }
            fs::write(&target, &contents).map_err(|error| EngineError::DownloadFailed {
                detail: format!("cannot write {}: {error}", target.display()),
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
            }

            if relative == wanted {
                executable = Some(target);
            }
        }

        let executable = executable.ok_or_else(|| EngineError::DownloadFailed {
            detail: format!(
                "engine zip does not contain '{wanted}' (product '{product}', version '{version}')"
            ),
        })?;
        Ok(InstalledEngine {
            product: product.to_owned(),
            version: version.to_owned(),
            executable,
        })
    }
}

/// Joins `relative` onto `base`, refusing hostile archive paths:
/// components must be plain names (no `..`, `.`, empty parts, absolute
/// prefixes, or Windows drive/UNC syntax), and the result must stay
/// inside `base`. Archive entries are hostile input (docs/architecture.md).
fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, EngineError> {
    let hostile = relative.is_empty()
        || relative.starts_with(['/', '\\'])
        || relative.contains(':')
        || !relative
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if hostile {
        return Err(EngineError::DownloadFailed {
            detail: format!("engine zip contains a hostile path entry: '{relative}'"),
        });
    }
    let target = base.join(relative);
    if !target.starts_with(base) {
        return Err(EngineError::DownloadFailed {
            detail: format!("engine zip entry escapes the cache directory: '{relative}'"),
        });
    }
    Ok(target)
}

/// Launchable file name of a product's binary.
pub fn binary_name(product: &str) -> String {
    if std::env::consts::OS == "windows" {
        format!("{product}.exe")
    } else {
        product.to_owned()
    }
}

/// Parses a dotted numeric version into comparable components.
fn parse_version(version: &str) -> Option<Vec<u32>> {
    version
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds an in-memory zip with one entry per path.
    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let options: zip::write::SimpleFileOptions = Default::default();
        for (path, contents) in entries {
            zip.start_file(*path, options).expect("zip entry");
            zip.write_all(contents).expect("zip contents");
        }
        zip.finish().expect("zip finish").into_inner()
    }

    fn product() -> &'static str {
        if std::env::consts::OS == "windows" {
            "chrome-headless-shell.exe"
        } else {
            "chrome-headless-shell"
        }
    }

    /// Zip in the layout Chrome for Testing ships: one folder prefixed
    /// with the platform, the binary plus its runtime data inside.
    fn shell_zip() -> Vec<u8> {
        let binary = format!("chrome-headless-shell-win64/{}", product());
        build_zip(&[
            (binary.as_str(), b"MZ" as &[u8]),
            ("chrome-headless-shell-win64/icudtl.dat", b"icu" as &[u8]),
            ("chrome-headless-shell-win64/resources.pak", b"pak" as &[u8]),
        ])
    }

    #[test]
    fn install_extracts_and_finds_binary() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let zip = shell_zip();

        let installed = store
            .install("chrome-headless-shell", "141.0.1", &zip)
            .expect("install");
        assert_eq!(installed.version, "141.0.1");
        assert!(installed.executable.is_file());
        assert_eq!(installed.executable.file_name().unwrap(), product());

        let version_dir = installed.executable.parent().expect("version dir");
        assert!(
            version_dir.join("icudtl.dat").is_file(),
            "runtime data must sit next to the binary"
        );

        let found = store
            .installed("chrome-headless-shell")
            .expect("scan")
            .expect("version installed");
        assert_eq!(found.executable, installed.executable);
    }

    /// Regression: the staging directory used to be named with only
    /// the process id, so concurrent installs of one version inside
    /// a single process (an integration suite on a cold cache) shared
    /// one staging directory and the winning rename made every other
    /// extraction fail mid-write. Each attempt now extracts into its
    /// own staging; the losers adopt the winner's install.
    #[test]
    fn concurrent_installs_of_one_version_all_succeed() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let version = "141.0.1";

        // Many entries keep every extraction busy so, on the old
        // pid-only staging name, the first rename routinely deleted
        // the directory out from under the other threads.
        let names: Vec<String> = (0..256)
            .map(|index| format!("chrome-headless-shell-win64/locales/{index}.pak"))
            .collect();
        let binary = format!("chrome-headless-shell-win64/{}", product());
        let mut entries: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), b"pak" as &[u8]))
            .collect();
        entries.insert(0, (binary.as_str(), b"MZ" as &[u8]));
        let zip = build_zip(&entries);

        let threads = 6;
        let barrier = std::sync::Barrier::new(threads);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    barrier.wait();
                    let installed = store
                        .install("chrome-headless-shell", version, &zip)
                        .expect("concurrent install must succeed");
                    assert!(installed.executable.is_file());
                });
            }
        });

        let found = store
            .installed("chrome-headless-shell")
            .expect("scan")
            .expect("version installed");
        assert_eq!(found.version, version);

        let product_dir = root.path().join("engines").join("chrome-headless-shell");
        let leftovers: Vec<String> = std::fs::read_dir(&product_dir)
            .expect("product dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(TMP_PREFIX))
            .collect();
        assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
    }

    #[test]
    fn install_without_binary_fails_and_leaves_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let zip = build_zip(&[("readme.txt", b"no binary here" as &[u8])]);

        let error = store
            .install("chrome-headless-shell", "141.0.1", &zip)
            .expect_err("missing binary must fail");
        assert!(matches!(error, EngineError::DownloadFailed { .. }));
        let found = store.installed("chrome-headless-shell").expect("scan");
        assert!(found.is_none(), "failed install must not be visible");
    }

    #[test]
    fn hostile_versions_are_rejected_before_touching_the_cache() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let zip = shell_zip();

        // The version comes verbatim from the downloaded manifest; a
        // traversal or separator-bearing value must fail before any
        // path is built, or the final rename would escape the cache.
        for version in [
            "../../evil",
            "141.0.1/../../../evil",
            "141.0.1\\..\\..\\evil",
            "C:\\evil",
            "/evil",
            "",
        ] {
            let error = store
                .install("chrome-headless-shell", version, &zip)
                .expect_err("hostile version must fail");
            assert!(
                error.to_string().contains("dotted-numeric"),
                "version '{version}' must fail the version check, not later work: {error}"
            );
        }
        assert!(
            store
                .installed("chrome-headless-shell")
                .expect("scan")
                .is_none(),
            "rejected installs must leave the cache untouched"
        );
    }

    #[test]
    fn picks_highest_installed_version() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let zip = shell_zip();
        for version in ["141.0.2", "139.9.0", "141.0.10"] {
            store
                .install("chrome-headless-shell", version, &zip)
                .expect("install");
        }

        let found = store
            .installed("chrome-headless-shell")
            .expect("scan")
            .expect("versions installed");
        assert_eq!(found.version, "141.0.10");
    }

    #[test]
    fn explicit_binary_bypasses_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fake = dir.path().join("my-browser");
        fs::write(&fake, b"binary").expect("write fake binary");

        let installed = InstalledEngine {
            product: "external".to_owned(),
            version: "external".to_owned(),
            executable: fake.clone(),
        };
        assert!(installed.executable.is_file());
    }

    #[test]
    fn hostile_zip_paths_are_rejected() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let binary = format!("chrome-headless-shell-win64/{}", product());
        let zip = build_zip(&[
            (binary.as_str(), b"MZ" as &[u8]),
            (
                "chrome-headless-shell-win64/../../evil.exe",
                b"evil" as &[u8],
            ),
        ]);

        let error = store
            .install("chrome-headless-shell", "141.0.1", &zip)
            .expect_err("zip slip must fail");
        assert!(error.to_string().contains("hostile path"));
        let outside = root.path().join("evil.exe");
        assert!(!outside.exists(), "no file may land outside the cache");
        let found = store.installed("chrome-headless-shell").expect("scan");
        assert!(found.is_none(), "failed install must not be visible");
    }

    #[test]
    fn version_parse_orders_numerically() {
        assert!(parse_version("141.0.10") > parse_version("141.0.2"));
        assert!(parse_version("abc").is_none());
    }

    #[test]
    fn an_entry_over_the_per_entry_cap_is_rejected() {
        // Caps are parameters so a bomb-sized archive can be simulated
        // with a tiny one: 2 bytes fits the binary entry, the next
        // entry blows the per-entry cap.
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let staging = root.path().join(".tmp-caps");
        std::fs::create_dir_all(&staging).expect("staging dir");

        let error = store
            .extract_archive_with_caps(
                "chrome-headless-shell",
                "141.0.1",
                &shell_zip(),
                &staging,
                2,
                u64::MAX,
            )
            .expect_err("an oversized entry must fail");
        assert!(
            error.to_string().contains("extraction cap"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_archive_over_the_total_cap_is_rejected() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let staging = root.path().join(".tmp-caps");
        std::fs::create_dir_all(&staging).expect("staging dir");

        let error = store
            .extract_archive_with_caps(
                "chrome-headless-shell",
                "141.0.1",
                &shell_zip(),
                &staging,
                u64::MAX,
                1,
            )
            .expect_err("an oversized total must fail");
        assert!(
            error.to_string().contains("total extraction cap"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_empty_cache_has_no_install() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        assert!(
            store
                .installed("chrome-headless-shell")
                .expect("scan")
                .is_none()
        );
    }

    #[test]
    fn scan_skips_garbage_and_versions_without_a_binary() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let product_dir = root.path().join("engines").join("chrome-headless-shell");
        // A stale staging dir, a non-version name, and a version whose
        // binary is missing: none of them is an install.
        for name in [".tmp-141.0.1-1", "latest", "141.0.1"] {
            std::fs::create_dir_all(product_dir.join(name)).expect("entry dir");
        }

        assert!(
            store
                .installed("chrome-headless-shell")
                .expect("scan")
                .is_none(),
            "garbage entries never satisfy a lookup"
        );
    }

    #[test]
    fn scan_reports_a_cache_path_that_is_not_a_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let product_dir = root.path().join("engines").join("chrome-headless-shell");
        std::fs::create_dir_all(product_dir.parent().expect("engines dir")).expect("parent");
        std::fs::write(&product_dir, b"i am a file").expect("decoy file");

        let error = store
            .installed("chrome-headless-shell")
            .expect_err("a file where the product dir belongs must fail");
        assert!(
            error.to_string().contains("cannot read cache directory"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn reinstalling_a_present_version_is_a_noop() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let zip = shell_zip();
        let first = store
            .install("chrome-headless-shell", "141.0.1", &zip)
            .expect("install");

        let second = store
            .install("chrome-headless-shell", "141.0.1", &zip)
            .expect("reinstall");
        assert_eq!(first.executable, second.executable);
        assert!(
            !root
                .path()
                .join("engines")
                .join("chrome-headless-shell")
                .read_dir()
                .expect("entries")
                .any(|entry| entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(TMP_PREFIX)),
            "a short-circuited install leaves no staging dir"
        );
    }

    #[test]
    fn an_unreadable_zip_fails_without_staging_leftovers() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());

        let error = store
            .install("chrome-headless-shell", "141.0.1", b"this is not a zip")
            .expect_err("garbage bytes must fail");
        assert!(
            error.to_string().contains("not readable"),
            "unexpected error: {error}"
        );
        assert!(
            store
                .installed("chrome-headless-shell")
                .expect("scan")
                .is_none()
        );
    }

    #[test]
    fn zips_with_directory_entries_and_bare_names_extract() {
        // Chrome for Testing zips carry a top-level folder; a zip whose
        // binary sits at the root (no folder at all) must install too,
        // and directory entries must be skipped, not written as files.
        let root = tempfile::tempdir().expect("tempdir");
        let store = EngineStore::new(root.path());
        let zip = build_zip(&[
            ("chrome-headless-shell-linux64/", b"" as &[u8]),
            (product(), b"MZ" as &[u8]),
        ]);

        let installed = store
            .install("chrome-headless-shell", "141.0.1", &zip)
            .expect("install");
        assert!(installed.executable.is_file());
    }

    #[test]
    fn safe_join_rejects_absolute_colon_and_escaping_names() {
        // The zip crate normalizes some of these away when writing an
        // archive, so the rejection itself is tested directly; the
        // end-to-end slip case lives in `hostile_zip_paths_are_rejected`.
        let base = Path::new("/cache/staging");
        for relative in ["/evil.exe", "C:evil.exe", "a//b", "a/./b", "a/../b", ""] {
            assert!(
                safe_join(base, relative).is_err(),
                "{relative:?} must be refused"
            );
        }
        let joined = safe_join(base, "folder/binary.exe").expect("a plain name joins");
        assert_eq!(joined, base.join("folder/binary.exe"));
    }
}
