//! `rutter read <url>`: one-shot scrape.
//!
//! Boundary: navigate once, print the page's readable content as a
//! markdown document to stdout, exit. No sessions, no recovery, no
//! policy — those belong to the serve and browse modes. stdout carries
//! only the readout; progress and errors go to stderr. The engine
//! lifecycle belongs to [`crate::one_shot`]; what is this mode's own
//! is which script runs and how its response becomes a readout.

use rutter_observe::{read_from_response, reader_script};

use crate::config::Settings;
use crate::error::CliError;
use crate::one_shot;

/// Runs the read mode to completion.
pub async fn run(settings: &Settings, url: &str) -> Result<(), CliError> {
    let readout = one_shot::drive(settings, url, |page, _effective_url| {
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
