//! Parses the Chrome for Testing last-known-good manifest.
//!
//! Boundary: manifest JSON to artifact metadata only. Downloading,
//! extraction, and cache layout live in sibling modules; the manifest
//! endpoint is a stable Google-published document, never computed here.

use serde_json::Value;

use crate::error::EngineError;

/// Default endpoint publishing the last-known-good version per channel
/// with download URLs for every platform.
pub const MANIFEST_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

/// Environment variable overriding [`MANIFEST_URL`], so an operator
/// whose hosts cannot reach Google can point rutter at a mirror that
/// republishes the same document. It names where the version pointer
/// is read from and nothing else: the artifact URL it resolves to is
/// still pinned to [`ARTIFACT_HOST`] over TLS, so a mirror cannot move
/// the downloaded binary off the Chrome for Testing storage host.
const MANIFEST_URL_ENV: &str = "RUTTER_ENGINE_MANIFEST_URL";

/// Host every artifact URL must live on. The manifest is the trust
/// anchor for which version to fetch; the URL it names is code rutter
/// executes, so without this pin a compromised manifest source could
/// redirect that download to an arbitrary server. Chrome for Testing
/// artifacts are published on this host only.
const ARTIFACT_HOST: &str = "storage.googleapis.com";

/// The manifest endpoint to fetch: the [`MANIFEST_URL_ENV`] override
/// when it is set to something, the published default otherwise.
pub fn manifest_url() -> String {
    resolve_manifest_url(std::env::var(MANIFEST_URL_ENV).ok().as_deref())
}

/// Resolves the endpoint from an already-read override value. Split
/// from the environment read so the rule is testable without mutating
/// process state; an absent or blank override keeps the default.
fn resolve_manifest_url(override_url: Option<&str>) -> String {
    override_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(MANIFEST_URL)
        .to_owned()
}

/// One downloadable engine binary from the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineArtifact {
    /// Engine version, for example `141.0.7390.78`.
    pub version: String,
    /// Download URL of the platform zip.
    pub url: String,
}

/// Maps the host platform to the manifest's platform identifier.
pub fn platform_for_host() -> Result<&'static str, EngineError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux64"),
        ("linux", "aarch64") => Err(unsupported("linux/aarch64")),
        ("macos", "x86_64") => Ok("mac-x64"),
        ("macos", "aarch64") => Ok("mac-arm64"),
        ("windows", "x86_64") => Ok("win64"),
        ("windows", "aarch64") => Ok("win64"),
        (os, arch) => Err(unsupported(&format!("{os}/{arch}"))),
    }
}

fn unsupported(platform: &str) -> EngineError {
    EngineError::DownloadFailed {
        detail: format!("no Chrome for Testing build for platform '{platform}'"),
    }
}

/// Extracts the stable-channel artifact for one product and platform.
pub fn parse_stable_artifact(
    manifest: &Value,
    product: &str,
    platform: &str,
) -> Result<EngineArtifact, EngineError> {
    let stable = manifest
        .get("channels")
        .and_then(|channels| channels.get("Stable"))
        .ok_or_else(|| malformed("manifest has no Stable channel"))?;

    let version = stable
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("manifest has no Stable version"))?
        .to_owned();

    let downloads = stable
        .get("downloads")
        .and_then(|downloads| downloads.get(product))
        .and_then(Value::as_array)
        .ok_or_else(|| malformed(&format!("manifest has no downloads for '{product}'")))?;

    let url = downloads
        .iter()
        .filter_map(|entry| {
            let entry_platform = entry.get("platform").and_then(Value::as_str)?;
            let entry_url = entry.get("url").and_then(Value::as_str)?;
            (entry_platform == platform).then_some(entry_url)
        })
        .next()
        .ok_or_else(|| {
            malformed(&format!(
                "manifest has no '{product}' download for '{platform}'"
            ))
        })?
        .to_owned();

    // The manifest itself is fetched over TLS, and the artifact URL it
    // names must stay on TLS *and* on the Chrome for Testing storage
    // host: the transfer is code that rutter executes, so a downgraded
    // scheme or a redirected host is malformed rather than followed.
    let host = https_host(&url)
        .ok_or_else(|| malformed(&format!("manifest artifact URL is not https: '{url}'")))?;
    if host != ARTIFACT_HOST {
        return Err(malformed(&format!(
            "manifest artifact URL must live on '{ARTIFACT_HOST}', not '{host}'"
        )));
    }

    Ok(EngineArtifact { version, url })
}

/// Extracts the host of an https URL; the caller has already ruled on
/// the scheme, so anything without the https authority is rejected.
fn https_host(url: &str) -> Option<&str> {
    let authority = url
        .strip_prefix("https://")?
        .split(['/', '?', '#'])
        .next()?;
    // A port is legal URL syntax but never part of the host comparison;
    // artifact URLs do not carry one, so stripping is only hardening.
    let host = authority
        .rsplit_once(':')
        .map(|(host, _port)| host)
        .unwrap_or(authority);
    (!host.is_empty()).then_some(host)
}

fn malformed(detail: &str) -> EngineError {
    EngineError::DownloadFailed {
        detail: format!("malformed engine manifest: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({
            "timestamp": "2026-09-10T00:06:44.697Z",
            "channels": {
                "Stable": {
                    "channel": "Stable",
                    "version": "141.0.7390.78",
                    "revision": "f00b4r",
                    "downloads": {
                        "chrome": [
                            { "platform": "linux64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.78/linux64/chrome-linux64.zip" },
                            { "platform": "win64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.78/win64/chrome-win64.zip" }
                        ],
                        "chrome-headless-shell": [
                            { "platform": "linux64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.78/linux64/chrome-headless-shell-linux64.zip" },
                            { "platform": "mac-arm64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.78/mac-arm64/chrome-headless-shell-mac-arm64.zip" },
                            { "platform": "win64", "url": "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.78/win64/chrome-headless-shell-win64.zip" }
                        ]
                    }
                }
            }
        })
    }

    #[test]
    fn parses_stable_artifact_for_platform() {
        let artifact = parse_stable_artifact(&fixture(), "chrome-headless-shell", "win64")
            .expect("fixture must parse");
        assert_eq!(artifact.version, "141.0.7390.78");
        assert_eq!(
            artifact.url,
            "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.78/win64/chrome-headless-shell-win64.zip"
        );
    }

    #[test]
    fn foreign_artifact_hosts_are_rejected() {
        let mut manifest = fixture();
        // The win64 entry is the one the parse below selects.
        manifest["channels"]["Stable"]["downloads"]["chrome-headless-shell"][2]["url"] =
            json!("https://mirror.attacker.example/chrome-headless-shell-win64.zip");
        let error = parse_stable_artifact(&manifest, "chrome-headless-shell", "win64")
            .expect_err("a redirected host must fail");
        assert!(
            error.to_string().contains("storage.googleapis.com"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn missing_platform_is_a_download_error() {
        let error = parse_stable_artifact(&fixture(), "chrome-headless-shell", "linux32")
            .expect_err("unknown platform must fail");
        assert!(matches!(error, EngineError::DownloadFailed { .. }));
    }

    #[test]
    fn plaintext_artifact_urls_are_rejected() {
        let mut manifest = fixture();
        // The win64 entry is the one the parse below selects.
        manifest["channels"]["Stable"]["downloads"]["chrome-headless-shell"][2]["url"] =
            json!("http://storage.example.com/shell-win64.zip");
        let error = parse_stable_artifact(&manifest, "chrome-headless-shell", "win64")
            .expect_err("http artifact must fail");
        assert!(
            error.to_string().contains("not https"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn malformed_manifest_is_a_download_error() {
        let error = parse_stable_artifact(&json!({}), "chrome-headless-shell", "win64")
            .expect_err("empty manifest must fail");
        assert!(
            error.to_string().contains("Stable channel"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn host_extraction_keeps_hostile_authorities_verbatim() {
        assert_eq!(
            https_host("https://storage.googleapis.com/a.zip"),
            Some("storage.googleapis.com")
        );
        assert_eq!(
            https_host("https://storage.googleapis.com:443/a.zip"),
            Some("storage.googleapis.com")
        );
        assert_eq!(https_host("http://storage.googleapis.com/a.zip"), None);
        assert_eq!(https_host("https:///a.zip"), None);
        // A userinfo trick keeps the whole authority, which the host
        // equality check then rejects.
        assert_eq!(
            https_host("https://storage.googleapis.com@evil.example/a.zip"),
            Some("storage.googleapis.com@evil.example")
        );
    }

    #[test]
    fn host_platform_is_recognized() {
        let platform = platform_for_host().expect("host must be supported in tests");
        assert!(["linux64", "mac-x64", "mac-arm64", "win64"].contains(&platform));
    }

    #[test]
    fn an_unset_or_blank_override_keeps_the_published_endpoint() {
        assert_eq!(resolve_manifest_url(None), MANIFEST_URL);
        assert_eq!(resolve_manifest_url(Some("")), MANIFEST_URL);
        assert_eq!(resolve_manifest_url(Some("   ")), MANIFEST_URL);
        // The live path with no variable in the environment must agree
        // with the pure rule above.
        assert_eq!(manifest_url(), resolve_manifest_url(None));
    }

    #[test]
    fn an_override_replaces_the_endpoint() {
        assert_eq!(
            resolve_manifest_url(Some("http://mirror.internal/cft.json")),
            "http://mirror.internal/cft.json"
        );
        // Surrounding whitespace is a copy-paste artifact, not part of
        // the URL the fetcher will be handed.
        assert_eq!(
            resolve_manifest_url(Some("  https://mirror.internal/cft.json \n")),
            "https://mirror.internal/cft.json"
        );
    }
}
