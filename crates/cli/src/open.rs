//! `rutter open <url>`: one-shot diagnostic.
//!
//! Boundary: navigate once, print one snapshot to stdout, exit. No
//! sessions, no recovery, no policy — those belong to the serve and
//! browse modes. stdout carries only the snapshot; progress and errors
//! go to stderr. This mode resolves the executable; the launch,
//! navigate and shutdown lifecycle belongs to [`crate::one_shot`].
//! What is this mode's own is which script runs and how its response
//! becomes a snapshot.

use rutter_observe::{serializer_script, snapshot_from_response};

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;
use crate::one_shot;

/// Runs the open mode to completion.
pub async fn run(settings: &Settings, url: &str) -> Result<(), CliError> {
    let engine = launcher::headless_launcher(settings).await?;
    let snapshot = one_shot::drive(settings, &engine, url, |page, effective_url| {
        Box::pin(async move {
            let raw = page.evaluate(serializer_script()).await?;
            // The snapshot names the page it was read from, so the
            // effective URL is part of the conversion.
            Ok(snapshot_from_response(effective_url, &raw))
        })
    })
    .await?;
    println!("{}", crate::render_marked(&snapshot, snapshot.truncated));
    Ok(())
}
