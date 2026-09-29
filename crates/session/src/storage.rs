//! Storage state capture and persistence: cookies plus localStorage.
//!
//! Boundary: snapshotting the session state and its
//! JSON form. File locations and write timing are the caller's; cookie
//! transport is the context's. localStorage dumps and restores run as
//! page scripts owned by `rutter-observe`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rutter_core::cookie::Cookie;
use serde::{Deserialize, Serialize};

use rutter_engine::context::ContextHandle;
use rutter_engine::page::PageHandle;
use rutter_observe::{storage_dump_script, storage_restore_script};

/// Disambiguates staging files between writes in the same process: the
/// process id alone collides when two overlapping actions persist the
/// same session's state at once, and the writers would then interleave
/// into one temporary — publishing a mixture, or losing the rename.
static WRITE_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// Everything needed to rebuild a session's login state.
///
/// Both fields default on read: a file written by a build that knew
/// only one of them — or by a build that knew one this one does not —
/// keeps the half it does carry instead of failing the whole parse and
/// reading back as an empty state, which is the one outcome the atomic
/// write exists to prevent ("silently drop the session's login").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageState {
    /// Context-scoped cookies.
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    /// localStorage per origin.
    #[serde(default)]
    pub origins: Vec<OriginStorage>,
}

/// One origin's localStorage entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginStorage {
    /// Origin, for example `https://example.com`.
    pub origin: String,
    /// Key-value pairs as stored.
    pub entries: Vec<(String, String)>,
}

impl StorageState {
    /// Captures cookies for the context plus localStorage for every
    /// open page's origin. Pages that do not answer contribute
    /// nothing; capture never fails a session.
    pub async fn capture(
        context: &dyn ContextHandle,
        pages: &[(String, Arc<dyn PageHandle>)],
    ) -> Self {
        let cookies = context.cookies().await.unwrap_or_default();
        let mut origins = Vec::new();
        for (current_origin, page) in pages {
            if origins
                .iter()
                .any(|existing: &OriginStorage| existing.origin == *current_origin)
            {
                continue;
            }
            if let Some(entries) = page
                .evaluate(&storage_dump_script())
                .await
                .ok()
                .and_then(|value| parse_dump(&value, current_origin))
            {
                origins.push(entries);
            }
        }
        Self { cookies, origins }
    }

    /// Restores cookies context-wide and localStorage on pages whose
    /// origin matches an entry. Best effort: failures degrade to fewer
    /// restored entries, never an error.
    pub async fn restore(
        &self,
        context: &dyn ContextHandle,
        pages: &[(String, Arc<dyn PageHandle>)],
    ) {
        if !self.cookies.is_empty() {
            let _ = context.set_cookies(&self.cookies).await;
        }
        self.restore_origins(pages).await;
    }

    /// Restores only the localStorage side, on pages whose origin
    /// matches an entry. Split out because cookies are context-wide:
    /// a caller replaying them once (recovery) must not repeat that
    /// engine round trip for every page it rebuilds. Best effort, like
    /// [`StorageState::restore`].
    pub async fn restore_origins(&self, pages: &[(String, Arc<dyn PageHandle>)]) {
        for (origin, page) in pages {
            let Some(entries) = self
                .origins
                .iter()
                .find(|existing| &existing.origin == origin)
            else {
                continue;
            };
            let Ok(json) = serde_json::to_string(
                &entries
                    .entries
                    .iter()
                    .cloned()
                    .collect::<std::collections::HashMap<String, String>>(),
            ) else {
                continue;
            };
            let _ = page.evaluate(&storage_restore_script(&json)).await;
        }
    }

    /// Writes the state as JSON; the session's persistence file. The
    /// write is atomic: the JSON lands in a sibling temporary file that
    /// replaces the real one in one rename, so a crash or a full disk
    /// can never leave a half-written file (which would read back as an
    /// empty state and silently drop the session's login). Each write
    /// stages under its own name, so two of them racing on one file
    /// cannot interleave into one temporary. The file holds cookies and
    /// localStorage — secrets — so it is created owner-only on Unix
    /// instead of inheriting the umask default.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A serialization failure must fail the write, not rename an
        // empty state over the persisted one.
        let json = serde_json::to_string(self).map_err(std::io::Error::from)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("storage.json");
        let staging = path.with_file_name(format!(
            ".{file_name}.tmp-{}-{}",
            std::process::id(),
            WRITE_ATTEMPT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = std::fs::File::options();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Mode applies at creation: the secrets never exist on disk
            // with the umask default (group/world readable).
            options.mode(0o600);
        }
        let write_result = options
            .open(&staging)
            .and_then(|mut file| std::io::Write::write_all(&mut file, json.as_bytes()));
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
        match publish(|| std::fs::rename(&staging, path)) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&staging);
                Err(error)
            }
        }
    }

    /// Reads a previously written state; a missing or corrupt file is
    /// an empty state.
    pub fn read(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }
}

/// How many times a publish is attempted before the failure is real.
///
/// The backoff tops out at 15 ms in total, so a transient refusal
/// costs less than one poll interval while a genuinely unwritable
/// destination still answers promptly.
const PUBLISH_ATTEMPTS: u32 = 5;

/// Moves the staged file into place, retrying the refusals a
/// concurrent publish causes.
///
/// Windows answers a rename onto a destination another thread is
/// renaming at that same moment with ERROR_ACCESS_DENIED or
/// ERROR_SHARING_VIOLATION. The publish is atomic either way, so the
/// loser is a timing victim, not a failed write: reporting it as one
/// leaves the caller believing a perfectly readable file is behind and
/// rewriting it on every later capture. Every other failure — a full
/// disk, a read-only directory, a missing staging file — answers on
/// the first attempt.
fn publish(mut rename: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match rename() {
            Ok(()) => return Ok(()),
            Err(error) => {
                attempt += 1;
                if attempt >= PUBLISH_ATTEMPTS || !is_transient_publish_error(&error) {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(1u64 << (attempt - 1)));
            }
        }
    }
}

/// Whether a publish failure is the refusal a concurrent publish
/// causes rather than a real one: ERROR_ACCESS_DENIED (5) and
/// ERROR_SHARING_VIOLATION (32). `std::io::ErrorKind` has no variant
/// for either, so the raw OS code is what names them. Unix failures
/// carry no OS code at all, so nothing there is ever retried.
fn is_transient_publish_error(error: &std::io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| code == 5 || code == 32)
}

/// Parses the dump script's answer into an origin storage entry.
fn parse_dump(value: &serde_json::Value, origin: &str) -> Option<OriginStorage> {
    let object = value.as_object()?;
    let data = object.get("data")?.as_object()?;
    let entries = data
        .iter()
        .filter_map(|(key, value)| {
            let value = value.as_str()?;
            Some((key.clone(), value.to_owned()))
        })
        .collect();
    Some(OriginStorage {
        origin: origin.to_owned(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_json() {
        let state = StorageState {
            cookies: vec![Cookie {
                name: "session".to_owned(),
                value: "42".to_owned(),
                domain: "example.com".to_owned(),
                path: Some("/".to_owned()),
                secure: true,
                http_only: true,
                same_site: None,
                expires: Some(1_800_000_000.0),
            }],
            origins: vec![OriginStorage {
                origin: "https://example.com".to_owned(),
                entries: vec![("token".to_owned(), "abc".to_owned())],
            }],
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/storage.json");
        state.write(&path).expect("write");
        let back = StorageState::read(&path);
        assert_eq!(back, state);
    }

    #[test]
    fn missing_files_read_as_empty_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = StorageState::read(&dir.path().join("missing.json"));
        assert!(state.cookies.is_empty());
        assert!(state.origins.is_empty());
    }

    #[test]
    fn a_state_missing_one_half_keeps_the_other() {
        // Forward compatibility of the on-disk format. `read` answers an
        // empty state for anything it cannot parse, and an empty state is
        // the outcome the atomic write exists to prevent — the session
        // comes back logged out with no complaint. A file written by a
        // build that knew only one of the two halves must therefore keep
        // the half it does carry instead of failing the whole parse.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("partial.json");

        std::fs::write(
            &path,
            r#"{"cookies":[{"name":"session","value":"42","domain":"example.com",
               "path":null,"secure":false,"http_only":false,"same_site":null,"expires":null}]}"#,
        )
        .expect("write cookies only");
        let cookies_only = StorageState::read(&path);
        assert_eq!(cookies_only.cookies.len(), 1, "the cookies survive");
        assert!(cookies_only.origins.is_empty(), "no origins were stored");

        std::fs::write(
            &path,
            r#"{"origins":[{"origin":"https://example.com","entries":[["token","abc"]]}]}"#,
        )
        .expect("write origins only");
        let origins_only = StorageState::read(&path);
        assert!(origins_only.cookies.is_empty(), "no cookies were stored");
        assert_eq!(origins_only.origins.len(), 1, "the localStorage survives");
        assert_eq!(origins_only.origins[0].entries[0].1, "abc");

        // A file that carries neither key is still the empty state, and
        // one whose values are of the wrong type still fails loudly
        // enough to read as empty — the default never invents content.
        std::fs::write(&path, "{}").expect("write empty object");
        assert_eq!(StorageState::read(&path), StorageState::default());
        std::fs::write(&path, r#"{"cookies":"not a list"}"#).expect("write junk");
        assert_eq!(StorageState::read(&path), StorageState::default());
    }

    /// One state whose serialized length grows with `padding`, so the
    /// concurrent writers below differ by whole kilobytes and an
    /// interleaved mixture can never be mistaken for either one.
    fn padded_state(marker: &str, padding: usize) -> StorageState {
        StorageState {
            cookies: Vec::new(),
            origins: vec![OriginStorage {
                origin: "https://example.com".to_owned(),
                entries: vec![
                    ("marker".to_owned(), marker.to_owned()),
                    ("pad".to_owned(), "p".repeat(padding)),
                ],
            }],
        }
    }

    #[test]
    fn concurrent_writes_of_one_file_all_succeed_and_stay_readable() {
        // The staging name used to be derived from the process id alone,
        // so two writers persisting the same session's file shared one
        // temporary: their writes interleaved into it and whichever
        // rename landed published the mixture, while the loser failed
        // renaming a file that no longer existed. A mixture reads back
        // as `StorageState::default()` — the silently empty login state
        // the atomic write exists to prevent. `Session::persist_storage`
        // takes `&self` and is reachable from two overlapping actions,
        // so one process really can write one file concurrently.
        const WRITERS: usize = 6;
        const ROUNDS: usize = 40;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let states: Vec<StorageState> = (0..WRITERS)
            .map(|index| padded_state(&format!("w{index}"), 4096 * index))
            .collect();

        let barrier = std::sync::Barrier::new(WRITERS);
        let errors: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = states
                .iter()
                .map(|state| {
                    let path = path.clone();
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let mut errors = Vec::new();
                        barrier.wait();
                        for _ in 0..ROUNDS {
                            if let Err(error) = state.write(&path) {
                                errors.push(error.to_string());
                            }
                        }
                        errors
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().expect("writer thread"))
                .collect()
        });

        assert!(
            errors.is_empty(),
            "every publish of one file must succeed: {errors:?}"
        );
        let back = StorageState::read(&path);
        assert!(
            states.contains(&back),
            "the published file must be exactly one writer's state, got {} origins",
            back.origins.len()
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
    }

    #[test]
    fn rewriting_replaces_the_file_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");

        let first = StorageState {
            cookies: vec![Cookie {
                name: "a".to_owned(),
                value: "1".to_owned(),
                domain: "example.com".to_owned(),
                path: None,
                secure: false,
                http_only: false,
                same_site: None,
                expires: None,
            }],
            origins: Vec::new(),
        };
        first.write(&path).expect("first write");

        let second = StorageState::default();
        second
            .write(&path)
            .expect("second write over an existing file");

        assert_eq!(StorageState::read(&path), second, "the rename replaced it");
        let left_behind: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(
            left_behind,
            vec![std::ffi::OsString::from("state.json")],
            "no staging files may survive"
        );
    }

    #[test]
    fn a_concurrent_publish_refusal_is_retried_not_reported() {
        // Windows refuses a rename onto a destination another thread is
        // renaming at that moment. The publish is atomic, so the loser
        // must retry: answering "the write failed" tells the session its
        // file is behind and rewrites it on every later capture.
        let mut attempts = 0;
        let published = publish(|| {
            attempts += 1;
            if attempts < 3 {
                Err(std::io::Error::from_raw_os_error(5))
            } else {
                Ok(())
            }
        });
        assert!(published.is_ok(), "a transient refusal is not a failure");
        assert_eq!(attempts, 3, "the refusal is retried, not given up on");
    }

    #[test]
    fn a_real_publish_failure_answers_on_the_first_attempt() {
        for error in [
            std::io::Error::from(std::io::ErrorKind::NotFound),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            std::io::Error::from_raw_os_error(2),
        ] {
            let mut attempts = 0;
            let outcome = publish(|| {
                attempts += 1;
                Err(std::io::Error::from(error.kind()))
            });
            assert!(outcome.is_err());
            assert_eq!(attempts, 1, "only the sharing refusals are retried");
        }
    }

    #[test]
    fn an_unwaivable_publish_failure_stops_after_its_attempts() {
        let mut attempts = 0;
        let outcome = publish(|| {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(32))
        });
        assert!(outcome.is_err(), "the last refusal is still a failure");
        assert_eq!(
            attempts, PUBLISH_ATTEMPTS as usize,
            "the retry budget is bounded"
        );
    }

    #[test]
    fn only_the_sharing_refusals_count_as_transient() {
        assert!(is_transient_publish_error(
            &std::io::Error::from_raw_os_error(5)
        ));
        assert!(is_transient_publish_error(
            &std::io::Error::from_raw_os_error(32)
        ));
        // A Unix failure carries no OS code, so it is never retried.
        assert!(!is_transient_publish_error(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )));
        assert!(!is_transient_publish_error(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
        assert!(!is_transient_publish_error(
            &std::io::Error::from_raw_os_error(28)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn persistence_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("secrets.json");
        let state = StorageState {
            cookies: vec![Cookie {
                name: "session".to_owned(),
                value: "s3cret".to_owned(),
                domain: "example.com".to_owned(),
                path: None,
                secure: true,
                http_only: true,
                same_site: None,
                expires: None,
            }],
            origins: Vec::new(),
        };
        state.write(&path).expect("write");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "cookie persistence must not be group or world readable"
        );
    }
}
