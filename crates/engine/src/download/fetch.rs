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

/// Upper bound for one fetched document. Legit Chrome for Testing
/// artifacts stay far below this; the cap only bounds a hostile or
/// broken source so a compromised manifest cannot exhaust memory.
const MAX_FETCH_BYTES: u64 = 1024 * 1024 * 1024;

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
                Ok(response) => {
                    // An announced body over the cap fails fast: a retry
                    // would only pull the same oversized document again.
                    if response
                        .content_length()
                        .is_some_and(|length| length > MAX_FETCH_BYTES)
                    {
                        return Err(EngineError::DownloadFailed {
                            detail: format!(
                                "response from '{url}' is over the {MAX_FETCH_BYTES}-byte cap"
                            ),
                        });
                    }
                    match read_capped(response).await {
                        BodyOutcome::Done(bytes) => return Ok(bytes),
                        // An over-cap body is deterministic like a client
                        // error: retrying changes nothing.
                        BodyOutcome::OverCap => {
                            return Err(EngineError::DownloadFailed {
                                detail: format!(
                                    "response from '{url}' is over the {MAX_FETCH_BYTES}-byte cap"
                                ),
                            });
                        }
                        BodyOutcome::ReadFailed(message) => last_error = Some(message),
                    }
                }
                Err(error) => {
                    // Client errors are deterministic: retrying changes
                    // nothing, so fail fast. Server and transport errors
                    // stay retriable.
                    if error
                        .status()
                        .is_some_and(|status| status.is_client_error())
                    {
                        return Err(EngineError::DownloadFailed {
                            detail: format!("server rejected the request: {error}"),
                        });
                    }
                    last_error = Some(format!("server rejected the request: {error}"));
                }
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

/// Body read outcome: success, a transport failure (retriable), or a
/// body over the cap (deterministic, not retried).
enum BodyOutcome {
    Done(Vec<u8>),
    ReadFailed(String),
    OverCap,
}

/// Reads the response body in chunks, refusing anything over
/// [`MAX_FETCH_BYTES`]. The cap applies to the bytes actually received:
/// a hostile source cannot rely on the declared content length.
async fn read_capped(mut response: reqwest::Response) -> BodyOutcome {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let received = body.len() as u64 + chunk.len() as u64;
                if received > MAX_FETCH_BYTES {
                    return BodyOutcome::OverCap;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return BodyOutcome::Done(body),
            Err(error) => return BodyOutcome::ReadFailed(format!("cannot read body: {error}")),
        }
    }
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

    #[tokio::test]
    async fn oversized_content_length_fails_fast() {
        // A local server announcing a body over the cap: the fetch must
        // reject it from the declared length instead of draining it, and
        // must not retry.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                2 * 1024 * 1024 * 1024u64
            );
            let _ = socket.write_all(head.as_bytes()).await;
        });

        let error = fetch_bytes(&format!("http://127.0.0.1:{port}/shell.zip"))
            .await
            .expect_err("an oversized body must fail");
        server.await.expect("server task");
        assert!(
            error.to_string().contains("byte cap"),
            "unexpected error: {error}"
        );
    }
}
