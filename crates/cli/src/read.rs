//! `rutter read <url>`: one-shot scrape.
//!
//! Boundary: navigate once, print the page's readable content as a
//! markdown document to stdout, exit. No sessions, no recovery, no
//! policy — those belong to the serve and browse modes. stdout carries
//! only the readout; progress and errors go to stderr. This mode
//! resolves the executable; the launch, navigate and shutdown
//! lifecycle belongs to [`crate::one_shot`]. What is this mode's own
//! is which script runs and how its response becomes a readout.

use rutter_observe::{read_from_response, reader_script};

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;
use crate::one_shot;

/// Runs the read mode to completion.
pub async fn run(settings: &Settings, url: &str) -> Result<(), CliError> {
    let engine = launcher::headless_launcher(settings).await?;
    let readout = one_shot::drive(settings, &engine, url, |page, _effective_url| {
        Box::pin(async move {
            let raw = page.evaluate(reader_script()).await?;
            // A readout is scoped by the page's own content, not by the
            // URL it was reached through, so the URL plays no part.
            Ok(read_from_response(&raw))
        })
    })
    .await?;
    println!("{}", crate::render_marked(&readout, readout.truncated));
    Ok(())
}
