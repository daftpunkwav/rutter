//! Storage state capture and persistence: cookies plus localStorage.
//!
//! Boundary: snapshotting the session state (docs/sessions.md) and its
//! JSON form. File locations and write timing are the caller's; cookie
//! transport is the context's. localStorage dumps and restores run as
//! page scripts owned by `rutter-observe`.

use std::path::Path;
use std::sync::Arc;

use rutter_core::cookie::Cookie;
use serde::{Deserialize, Serialize};

use rutter_engine::context::ContextHandle;
use rutter_engine::page::PageHandle;
use rutter_observe::{storage_dump_script, storage_restore_script};

/// Everything needed to rebuild a session's login state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageState {
    /// Context-scoped cookies.
    pub cookies: Vec<Cookie>,
    /// localStorage per origin.
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
    /// restored entries, never an error (docs/architecture.md).
    pub async fn restore(
        &self,
        context: &dyn ContextHandle,
        pages: &[(String, Arc<dyn PageHandle>)],
    ) {
        if !self.cookies.is_empty() {
            let _ = context.set_cookies(&self.cookies).await;
        }
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
    /// empty state and silently drop the session's login). The file
    /// holds cookies and localStorage — secrets — so it is created
    /// owner-only on Unix instead of inheriting the umask default.
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
        let staging = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
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
        match std::fs::rename(&staging, path) {
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
