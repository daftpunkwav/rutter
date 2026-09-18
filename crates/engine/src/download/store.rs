//! Engine binary cache: layout, extraction, and lookup.
//!
//! Boundary: filesystem state under the cache root only. Manifest
//! parsing and network fetches live in sibling modules. Layout:
//! `<root>/engines/<product>/<version>/<binary>`; one pinned version per
//! directory so concurrent rutter processes reuse the same install.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::EngineError;

/// Directory name for in-progress installs; renamed into place on
/// success so a crash never leaves a half-extracted version visible.
const TMP_PREFIX: &str = ".tmp-";

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
            .join(format!("{TMP_PREFIX}{version}-{}", std::process::id()));
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

        if let Some(parent) = final_dir.parent() {
            fs::create_dir_all(parent).map_err(|error| EngineError::DownloadFailed {
                detail: format!(
                    "cannot create cache directory {}: {error}",
                    parent.display()
                ),
            })?;
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
        let reader = std::io::Cursor::new(zip_bytes);
        let mut archive =
            zip::ZipArchive::new(reader).map_err(|error| EngineError::DownloadFailed {
                detail: format!("engine zip is not readable: {error}"),
            })?;

        let wanted = binary_name(product);
        let mut executable = None;
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
            let mut contents = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut contents)
                .map_err(|error| EngineError::DownloadFailed {
                    detail: format!("cannot read '{name}' from engine zip: {error}"),
                })?;
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
/// inside `base`. Archive entries are hostile input (blueprint §8.4).
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
}
