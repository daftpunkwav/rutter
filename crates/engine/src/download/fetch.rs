//! Timed, retried HTTP fetches for the engine downloader.
//!
//! Boundary: raw bytes from a URL. Every request carries a timeout and
//! bounded retries with the shared backoff; no URL is ever constructed
//! here — callers pass full endpoints from the manifest module.

use std::time::Duration;

use serde_json::Value;

use crate::backoff::Backoff;
use crate::error::EngineError;

/// Maximum attempts per fetch (first try plus retries).
const MAX_ATTEMPTS: u32 = 3;

/// Backoff between attempts: 2 s, 4 s, capped at 8 s.
fn retry_backoff() -> Backoff {
    Backoff::new(Duration::from_secs(2), Duration::from_secs(8))
}

/// Fetches a JSON document (the engine manifest); 30 s budget.
pub async fn fetch_json(url: &str) -> Result<Value, EngineError> {
    let body = fetch_with_budget(url, Duration::from_secs(30)).await?;
    serde_json::from_slice(&body).map_err(|error| EngineError::DownloadFailed {
        detail: format!("manifest is not valid JSON: {error}"),
    })
}

/// Fetches binary content (the engine zip); 10 minute budget.
pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>, EngineError> {
    fetch_with_budget(url, Duration::from_secs(600)).await
}

/// Performs one fetch with up to [`MAX_ATTEMPTS`] timed attempts.
async fn fetch_with_budget(url: &str, budget: Duration) -> Result<Vec<u8>, EngineError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(budget)
        .build()
        .map_err(|error| EngineError::DownloadFailed {
            detail: format!("cannot create HTTP client: {error}"),
        })?;

    let backoff = retry_backoff();
    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(backoff.delay(attempt - 1)).await;
        }
        match client.get(url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.bytes().await {
                    Ok(bytes) => return Ok(bytes.to_vec()),
                    Err(error) => last_error = Some(format!("cannot read body: {error}")),
                },
                Err(error) => last_error = Some(format!("server rejected the request: {error}")),
            },
            Err(error) => last_error = Some(format!("request failed: {error}")),
        }
    }

    Err(EngineError::DownloadFailed {
        detail: format!(
            "fetch of '{url}' failed after {MAX_ATTEMPTS} attempts: {}",
            last_error.unwrap_or_else(|| "unknown error".to_owned())
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_backoff_stays_capped() {
        let backoff = retry_backoff();
        assert_eq!(backoff.delay(0), Duration::from_secs(2));
        assert_eq!(backoff.delay(5), Duration::from_secs(8));
    }

    #[tokio::test]
    async fn unreachable_host_fails_with_download_error() {
        // Reserved documentation host: guaranteed not to resolve, so this
        // exercises the retry exhaustion path without network access.
        let error = fetch_json("https://invalid.invalid/manifest.json")
            .await
            .expect_err("unreachable host must fail");
        assert!(matches!(error, EngineError::DownloadFailed { .. }));
        assert!(error.to_string().contains("3 attempts"));
    }
}
