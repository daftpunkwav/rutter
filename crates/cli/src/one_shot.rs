//! The engine drive the one-shot modes share.
//!
//! Boundary: `open` and `read` differ only in which page script they
//! evaluate and how they turn the raw response into the thing they
//! print. Everything around that — resolve a headless engine, give it
//! a context and a page, navigate, and shut the engine down again — is
//! one job, and it is owned here so a mode cannot launch a browser and
//! then forget to close it. The modes keep what is theirs: the script,
//! the conversion, and the printing.

use std::future::Future;
use std::pin::Pin;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::page::PageHandle;
use rutter_engine::supervisor::EngineLauncher;

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;

/// What a one-shot mode does with the page it was given. The boxed
/// future is what lets the closure borrow the page: the drive owns the
/// engine, so the work must run inside it.
type PageUse<'page, T> = Pin<Box<dyn Future<Output = Result<T, CliError>> + Send + 'page>>;

/// Navigates a fresh engine to `url` and hands the first page to
/// `use_page`, which observes it and produces the mode's output.
///
/// The engine is shut down on every path — a failed resolve, a failed
/// launch, a failed navigation, a failing observation — because a
/// one-shot mode that cannot produce its output still must not leave a
/// browser process behind. The second argument is the effective URL
/// (what the page actually reached, which is not what was asked for
/// after a redirect).
pub(crate) async fn drive<T, F>(settings: &Settings, url: &str, use_page: F) -> Result<T, CliError>
where
    F: for<'page> FnOnce(&'page dyn PageHandle, &'page str) -> PageUse<'page, T>,
{
    let launcher = launcher::headless_launcher(settings).await?;
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let observed = async {
        let context = engine
            .create_context(ContextConfig {
                navigation_timeout: settings.navigation_timeout,
                ..ContextConfig::default()
            })
            .await?;
        let (_page_id, page) = context.open_page().await?;
        let effective_url = page.navigate(url).await?;
        use_page(page.as_ref(), &effective_url).await
    }
    .await;

    let _ = engine.shutdown().await;
    observed
}
