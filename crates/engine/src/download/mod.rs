//! Engine binary resolution: explicit path, cache, then download.
//!
//! Boundary: the `ensure` front door of engine acquisition. Resolution
//! order: a caller-supplied executable path wins (supporting system
//! browsers), then the pinned cache, then the Chrome for Testing
//! manifest over the network. Nothing else in the codebase launches or
//! downloads engines directly.

pub mod fetch;
pub mod manifest;
pub mod store;

use std::path::{Path, PathBuf};

pub use store::{EngineStore, InstalledEngine};

use crate::error::EngineError;

/// Which Chrome for Testing product to acquire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// The minimal headless shell; the default engine.
    ChromeHeadlessShell,
    /// The full browser; required for headed windows (browse mode).
    Chrome,
}

impl Product {
    /// Directory and manifest name of this product.
    pub fn name(self) -> &'static str {
        match self {
            Self::ChromeHeadlessShell => "chrome-headless-shell",
            Self::Chrome => "chrome",
        }
    }
}

/// Default cache root: `<OS cache dir>/rutter`, overridable via the
/// `RUTTER_CACHE_DIR` environment variable.
pub fn cache_root_default() -> Result<PathBuf, EngineError> {
    if let Ok(root) = std::env::var("RUTTER_CACHE_DIR") {
        return Ok(PathBuf::from(root));
    }
    let base = dirs::cache_dir().ok_or_else(|| EngineError::DownloadFailed {
        detail: "cannot determine the OS cache directory; set RUTTER_CACHE_DIR".to_owned(),
    })?;
    Ok(base.join("rutter"))
}

/// Locates a system-installed Chromium-family browser for headed runs.
///
/// Checks standard install locations only; no PATH search, no registry
/// probing. Hosts without any browser install let browse mode download
/// full Chrome instead.
pub fn discover_system_browser() -> Option<PathBuf> {
    system_browser_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

fn system_browser_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if std::env::consts::OS == "windows" {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(|name| std::env::var_os(name).map(PathBuf::from));
        for root in roots {
            paths.push(root.join(r"Google\Chrome\Application\chrome.exe"));
            paths.push(root.join(r"Microsoft\Edge\Application\msedge.exe"));
        }
    } else if std::env::consts::OS == "macos" {
        for path in [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ] {
            paths.push(PathBuf::from(path));
        }
    } else {
        for path in [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
        ] {
            paths.push(PathBuf::from(path));
        }
    }
    paths
}

/// Resolves a launchable engine binary.
///
/// `explicit` short-circuits everything: the path must exist and be a
/// file (its version reports as `external`). Otherwise the cache is
/// consulted, and only on a miss is the manifest fetched and the zip
/// downloaded. Progress goes to stderr; stdout stays reserved for data.
pub async fn ensure(
    product: Product,
    cache_root: &Path,
    explicit: Option<&Path>,
) -> Result<InstalledEngine, EngineError> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(EngineError::DownloadFailed {
                detail: format!("engine executable '{}' does not exist", path.display()),
            });
        }
        return Ok(InstalledEngine {
            product: "external".to_owned(),
            version: "external".to_owned(),
            executable: path.to_path_buf(),
        });
    }

    let store = EngineStore::new(cache_root);
    if let Some(installed) = store.installed(product.name())? {
        return Ok(installed);
    }

    let platform = manifest::platform_for_host()?;
    eprintln!(
        "rutter: fetching engine manifest for {} ({platform})",
        product.name()
    );
    let manifest_value = fetch::fetch_json(manifest::MANIFEST_URL).await?;
    let artifact = manifest::parse_stable_artifact(&manifest_value, product.name(), platform)?;

    // Unreachable in a single process (the cache was already a miss),
    // but a concurrent rutter may have installed this exact version
    // while we fetched the manifest; reuse it instead of re-downloading.
    if let Some(installed) = store.installed(product.name())? {
        if installed.version == artifact.version {
            return Ok(installed);
        }
    }

    eprintln!(
        "rutter: downloading {} {} ({platform}); the cache pins it until cleared",
        product.name(),
        artifact.version
    );
    let zip_bytes = fetch::fetch_bytes(&artifact.url).await?;
    let installed = store.install(product.name(), &artifact.version, &zip_bytes)?;
    eprintln!("rutter: engine ready at {}", installed.executable.display());
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_names_match_manifest() {
        assert_eq!(Product::ChromeHeadlessShell.name(), "chrome-headless-shell");
        assert_eq!(Product::Chrome.name(), "chrome");
    }

    #[tokio::test]
    async fn explicit_path_must_exist() {
        let error = ensure(
            Product::ChromeHeadlessShell,
            Path::new("unused-cache"),
            Some(Path::new("Z:/definitely/missing/browser")),
        )
        .await
        .expect_err("missing explicit path must fail");
        assert!(error.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn explicit_path_wins_without_cache_or_network() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fake = dir.path().join("browser");
        std::fs::write(&fake, b"binary").expect("write binary");

        let cache = tempfile::tempdir().expect("tempdir");
        let installed = ensure(Product::ChromeHeadlessShell, cache.path(), Some(&fake))
            .await
            .expect("explicit path must resolve");
        assert_eq!(installed.executable, fake);
        assert_eq!(installed.version, "external");
        assert!(
            !cache.path().join("engines").exists(),
            "explicit path must not touch the cache"
        );
    }
}
