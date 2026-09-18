//! `rutter` with no arguments: browse mode.
//!
//! Boundary: a headed engine window operated directly by a human, kept
//! alive by the supervisor until Ctrl-C. The supervision dashboard is
//! enabled through `rutter serve --dashboard`, not by this mode.

use rutter_engine::config::LaunchMode;
use rutter_engine::supervisor::Supervisor;

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;

/// Runs browse mode until the user interrupts it.
pub async fn run(settings: &Settings) -> Result<(), CliError> {
    let launcher = launcher::headed_launcher(settings).await?;
    let supervisor = Supervisor::new(std::sync::Arc::new(launcher), LaunchMode::Headed);
    supervisor.start().await?;
    println!(
        "rutter: browse mode is running; the engine window is yours.\
         \nPress Ctrl-C to exit."
    );

    tokio::signal::ctrl_c()
        .await
        .map_err(|error| CliError::Unavailable {
            mode: "browse".to_owned(),
            reason: format!("signal handling failed: {error}"),
        })?;

    println!("rutter: shutting down");
    supervisor.shutdown().await;
    Ok(())
}
