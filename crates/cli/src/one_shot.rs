//! The engine drive the one-shot modes share.
//!
//! Boundary: `open` and `read` differ only in which page script they
//! evaluate, how they turn the raw response into the thing they print,
//! and which headless engine they resolve. Everything around that —
//! launch the engine they resolved, give it a context and a page,
//! navigate, and shut the engine down again — is one job, and it is
//! owned here so a mode cannot launch a browser and then forget to
//! close it. The modes keep what is theirs: the executable, the
//! script, the conversion, and the printing.

use std::future::Future;
use std::pin::Pin;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::page::PageHandle;
use rutter_engine::supervisor::EngineLauncher;

use crate::config::Settings;
use crate::error::CliError;

/// What a one-shot mode does with the page it was given. The boxed
/// future is what lets the closure borrow the page: the drive owns the
/// engine, so the work must run inside it.
type PageUse<'page, T> = Pin<Box<dyn Future<Output = Result<T, CliError>> + Send + 'page>>;

/// Navigates a fresh engine to `url` and hands the first page to
/// `use_page`, which observes it and produces the mode's output.
///
/// The engine is shut down on every path — a failed launch, a failed
/// navigation, a failing observation — because a one-shot mode that
/// cannot produce its output still must not leave a browser process
/// behind. (A failed *resolve* cannot leak one: it happens before this
/// function is entered, in the mode's own `headless_launcher` call.)
/// The third argument is the effective URL (what the page actually
/// reached, which is not what was asked for after a redirect).
///
/// The launcher is a parameter rather than a `headless_launcher`
/// resolution of this crate's own: the shutdown contract is the whole
/// reason this function exists, and a test can only prove it against
/// an engine double. Resolving the executable stays in the mode.
pub(crate) async fn drive<T, F>(
    settings: &Settings,
    launcher: &dyn EngineLauncher,
    url: &str,
    use_page: F,
) -> Result<T, CliError>
where
    F: for<'page> FnOnce(&'page dyn PageHandle, &'page str) -> PageUse<'page, T>,
{
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

#[cfg(test)]
mod tests {
    //! The shutdown contract, against an engine double.
    //!
    //! The doc above promises the engine is closed on every path. That
    //! promise used to be untestable: `drive` resolved a real
    //! `CdpLauncher` of its own, so a test could only reach it by
    //! downloading a browser. The launcher is a parameter now, and
    //! these tests drive the whole chain — launcher, engine, context,
    //! page — with a double that records what it was asked to do.

    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use rutter_core::ids::{ContextId, PageId};
    use rutter_engine::context::ContextHandle;
    use rutter_engine::descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
    use rutter_engine::engine::Engine;
    use rutter_engine::error::EngineError;
    use rutter_engine::health::HealthReport;
    use rutter_engine::input::InputEvent;
    use rutter_engine::page::{ScreencastStream, Screenshot};
    use serde_json::Value;

    fn settings() -> Settings {
        Settings::resolve(None, Some(std::env::temp_dir()), Vec::new())
            .expect("settings with an explicit cache root")
    }

    fn unsupported(operation: &str) -> EngineError {
        EngineError::Unsupported {
            operation: operation.to_owned(),
            reason: "the one-shot double implements only what drive uses".to_owned(),
        }
    }

    /// What the double chain was asked to do, and what it refuses.
    #[derive(Default)]
    struct Log {
        shutdowns: AtomicUsize,
        navigations: AtomicUsize,
        evaluated: AtomicUsize,
        /// The effective URL `navigate` answers with, or a refusal.
        effective_url: Mutex<Option<Result<String, ()>>>,
        /// Whether `create_context` refuses.
        context_fails: AtomicBool,
    }

    impl Log {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn set_effective_url(&self, url: &str) {
            *self.effective_url.lock().unwrap() = Some(Ok(url.to_owned()));
        }

        fn fail_navigation(&self) {
            *self.effective_url.lock().unwrap() = Some(Err(()));
        }

        fn effective(&self) -> Result<String, EngineError> {
            match self
                .effective_url
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| Ok("https://asked.example".to_owned()))
            {
                Ok(url) => Ok(url),
                Err(()) => Err(EngineError::Terminated),
            }
        }
    }

    struct DoubleLauncher {
        log: Arc<Log>,
        /// When set, the launch itself fails and no engine exists.
        launch_fails: bool,
    }

    #[async_trait::async_trait]
    impl EngineLauncher for DoubleLauncher {
        fn describe(&self) -> String {
            "one-shot double".to_owned()
        }

        async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
            if self.launch_fails {
                return Err(EngineError::Terminated);
            }
            Ok(Arc::new(DoubleEngine {
                log: Arc::clone(&self.log),
            }))
        }
    }

    struct DoubleEngine {
        log: Arc<Log>,
    }

    #[async_trait::async_trait]
    impl Engine for DoubleEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                backend: EngineBackend::ChromiumHeadlessShell,
                version: "double".to_owned(),
                capabilities: EngineCapabilities {
                    headless: true,
                    headed: false,
                    screencast: false,
                    per_context_isolation: true,
                },
            }
        }

        async fn create_context(
            &self,
            _config: ContextConfig,
        ) -> Result<Arc<dyn ContextHandle>, EngineError> {
            if self.log.context_fails.load(Ordering::SeqCst) {
                return Err(EngineError::Terminated);
            }
            Ok(Arc::new(DoubleContext {
                log: Arc::clone(&self.log),
            }))
        }

        async fn health(&self) -> Result<HealthReport, EngineError> {
            Ok(HealthReport {
                healthy: true,
                backend_version: Some("double".to_owned()),
                detail: None,
            })
        }

        async fn shutdown(&self) -> Result<(), EngineError> {
            self.log.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct DoubleContext {
        log: Arc<Log>,
    }

    #[async_trait::async_trait]
    impl ContextHandle for DoubleContext {
        fn id(&self) -> ContextId {
            ContextId::new("ctx-double")
        }

        fn pages(&self) -> Vec<PageId> {
            Vec::new()
        }

        async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
            Ok((
                PageId::new("page-double"),
                Arc::new(DoublePage {
                    log: Arc::clone(&self.log),
                }),
            ))
        }

        fn page(&self, _id: PageId) -> Option<Arc<dyn PageHandle>> {
            None
        }

        async fn close_page(&self, _id: PageId) -> Result<(), EngineError> {
            Ok(())
        }

        async fn set_cookies(
            &self,
            _cookies: &[rutter_core::cookie::Cookie],
        ) -> Result<(), EngineError> {
            Ok(())
        }

        async fn cookies(&self) -> Result<Vec<rutter_core::cookie::Cookie>, EngineError> {
            Ok(Vec::new())
        }

        async fn close(&self) -> Result<(), EngineError> {
            Ok(())
        }
    }

    struct DoublePage {
        log: Arc<Log>,
    }

    #[async_trait::async_trait]
    impl PageHandle for DoublePage {
        async fn navigate(&self, _url: &str) -> Result<String, EngineError> {
            self.log.navigations.fetch_add(1, Ordering::SeqCst);
            self.log.effective()
        }

        async fn reload(&self) -> Result<(), EngineError> {
            Err(unsupported("reload"))
        }

        async fn go_back(&self) -> Result<String, EngineError> {
            Err(unsupported("go_back"))
        }

        async fn go_forward(&self) -> Result<String, EngineError> {
            Err(unsupported("go_forward"))
        }

        async fn evaluate(&self, _expression: &str) -> Result<Value, EngineError> {
            self.log.evaluated.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"version": 1, "truncated": false, "root": {"role": "root"}}))
        }

        async fn dispatch_input(&self, _event: InputEvent) -> Result<(), EngineError> {
            Err(unsupported("dispatch_input"))
        }

        async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
            Err(unsupported("capture_screenshot"))
        }

        async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
            Err(unsupported("start_screencast"))
        }
    }

    /// A mode's page work: evaluate once, hand the effective URL back.
    fn observe<'page>(
        page: &'page dyn PageHandle,
        effective_url: &'page str,
    ) -> PageUse<'page, String> {
        Box::pin(async move {
            page.evaluate("readout").await?;
            Ok(effective_url.to_owned())
        })
    }

    #[tokio::test]
    async fn a_successful_drive_evaluates_the_page_and_still_shuts_down() {
        let log = Log::new();
        log.set_effective_url("https://reached.example/final");
        let launcher = DoubleLauncher {
            log: Arc::clone(&log),
            launch_fails: false,
        };

        let observed = drive(&settings(), &launcher, "https://asked.example", observe)
            .await
            .expect("the drive succeeds");

        assert_eq!(observed, "https://reached.example/final");
        assert_eq!(log.navigations.load(Ordering::SeqCst), 1, "one navigation");
        assert_eq!(log.evaluated.load(Ordering::SeqCst), 1, "one evaluation");
        assert_eq!(
            log.shutdowns.load(Ordering::SeqCst),
            1,
            "the happy path closes the engine too"
        );
    }

    #[tokio::test]
    async fn a_failed_navigation_still_shuts_the_engine_down() {
        // The regression this pins: a one-shot that cannot produce its
        // output must not leave a browser process behind, and a failed
        // navigation is the path a bad URL or an offline host takes.
        let log = Log::new();
        log.fail_navigation();
        let launcher = DoubleLauncher {
            log: Arc::clone(&log),
            launch_fails: false,
        };

        let result = drive(&settings(), &launcher, "https://asked.example", observe).await;

        assert!(result.is_err(), "a failed navigation is reported");
        assert_eq!(
            log.evaluated.load(Ordering::SeqCst),
            0,
            "a page that never loaded is never evaluated"
        );
        assert_eq!(
            log.shutdowns.load(Ordering::SeqCst),
            1,
            "a failed navigation must not leak the engine"
        );
    }

    #[tokio::test]
    async fn a_failing_observation_still_shuts_the_engine_down() {
        // The `use_page` half is the mode's own code, and it can fail
        // on a script that threw or an envelope that would not parse.
        let log = Log::new();
        let launcher = DoubleLauncher {
            log: Arc::clone(&log),
            launch_fails: false,
        };

        let result = drive(
            &settings(),
            &launcher,
            "https://asked.example",
            |_page, _url| {
                Box::pin(async { Err::<String, _>(CliError::from(EngineError::Terminated)) })
            },
        )
        .await;

        assert!(result.is_err(), "the mode's failure is reported");
        assert_eq!(
            log.shutdowns.load(Ordering::SeqCst),
            1,
            "a failing observation must not leak the engine"
        );
    }

    #[tokio::test]
    async fn a_context_that_cannot_be_created_still_shuts_the_engine_down() {
        let log = Log::new();
        log.context_fails.store(true, Ordering::SeqCst);
        let launcher = DoubleLauncher {
            log: Arc::clone(&log),
            launch_fails: false,
        };

        let result = drive(&settings(), &launcher, "https://asked.example", observe).await;

        assert!(result.is_err(), "a context failure is reported");
        assert_eq!(
            log.shutdowns.load(Ordering::SeqCst),
            1,
            "the engine exists already, so it must be closed"
        );
    }

    #[tokio::test]
    async fn a_launch_that_never_produced_an_engine_closes_nothing() {
        // The other half of the contract: there is nothing to close,
        // and claiming a shutdown here would be a test that passes
        // because the counter was never wired.
        let log = Log::new();
        let launcher = DoubleLauncher {
            log: Arc::clone(&log),
            launch_fails: true,
        };

        let result = drive(&settings(), &launcher, "https://asked.example", observe).await;

        assert!(result.is_err(), "a launch failure is reported");
        assert_eq!(
            log.shutdowns.load(Ordering::SeqCst),
            0,
            "no engine was ever launched"
        );
    }
}
