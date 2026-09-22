//! Launcher producing CDP engines from a resolved browser executable.
//!
//! Boundary: the CDP registration point. A launcher pairs an executable
//! path with its backend identity (headless shell or full Chrome) and
//! produces engine instances; the binary decides which launcher to
//! build. Process supervision is the supervisor's concern, not this
//! factory's.
//!
//! rutter spawns the browser process itself and attaches via the
//! debugging port instead of letting chromiumoxide spawn it, for two
//! Windows facts the library's spawner cannot work around:
//!
//! - Launcher-style executables (Edge) relay to a child process and
//!   exit immediately, closing the stderr stream chromiumoxide parses
//!   the websocket address from. A loopback port polled through
//!   `json/version` is indifferent to who survives the spawn.
//! - chromiumoxide's spawner runs children inside a job object; full
//!   Chrome aborts startup there with exit code 21, while the headless
//!   shell tolerates it. A plain process spawn keeps both alive.
//!
//! The engine owns the spawned child so a dropped engine takes the
//! browser with it; graceful shutdown goes through the CDP
//! `Browser.close` command (see [`CdpEngine::shutdown`]), which also
//! reaches a browser whose intermediate launcher already exited. An
//! abnormal rutter kill can therefore leak a browser whose spawn
//! exited — the known cost of launcher-style executables.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chromiumoxide::Browser;
use futures::StreamExt;
use tokio::process::Child;

use rutter_engine::config::LaunchMode;
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::supervisor::EngineLauncher;

use crate::engine::{BrowserProcess, CdpEngine};
use crate::error;

/// Overall budget for one browser launch: process spawn, the debugging
/// endpoint coming up, and the websocket handshake. Child exit is not a
/// failure signal here — launcher-style executables exit by design — so
/// a wedged browser is detected by this deadline alone.
const LAUNCH_TIMEOUT: Duration = error::COMMAND_TIMEOUT;

/// One connection attempt's budget, so an endpoint that accepts but
/// never answers cannot stall the retry loop past its deadline.
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(1);

/// Pause between connection attempts.
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Automation defaults for the browser command line, ported from
/// chromiumoxide's `DEFAULT_ARGS` so the spawn rutter owns passes what
/// the library used to pass.
const AUTOMATION_ARGS: &[&str] = &[
    "--disable-background-networking",
    "--enable-features=NetworkService,NetworkServiceInProcess",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-breakpad",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--disable-default-apps",
    "--disable-dev-shm-usage",
    "--disable-features=TranslateUI",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--disable-sync",
    "--force-color-profile=srgb",
    "--metrics-recording-only",
    "--no-first-run",
    "--enable-automation",
    "--password-store=basic",
    "--use-mock-keychain",
    "--enable-blink-features=IdleDetection",
    "--lang=en_US",
];

/// Builds CDP engines from one executable.
pub struct CdpLauncher {
    executable: PathBuf,
    backend: EngineBackend,
    extra_args: Vec<String>,
    /// Whether headed mode should hide the browser shell (`--app`).
    /// Engines whose window is already a bare page surface — the
    /// Electron-based Rutter Browser — must not receive it: Electron
    /// reserves `--app` to mean "run this app" and would never start
    /// rutter's main.js.
    app_window: bool,
}

impl CdpLauncher {
    /// Creates a launcher for an executable and declares which backend
    /// it provides.
    pub fn new(executable: PathBuf, backend: EngineBackend) -> Self {
        Self {
            executable,
            backend,
            extra_args: Vec::new(),
            app_window: false,
        }
    }

    /// Marks headed mode as a chromeless app window (`--app`). Set by
    /// the CLI whenever it resolves the headed browser itself; an
    /// explicitly chosen engine binary keeps full control of its own
    /// window story.
    pub fn with_app_window(mut self, app_window: bool) -> Self {
        self.app_window = app_window;
        self
    }

    /// Passes arguments verbatim to the browser process, for example
    /// `--no-sandbox` in root containers or a proxy flag. Appended last,
    /// so a Chromium flag repeated here overrides the assembled default.
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Assembles the browser command line. rutter owns the debugging
    /// port and the profile directory unless the extras name them
    /// explicitly; a per-launch profile is what keeps the headed window
    /// free of bookmarks, history, and login state.
    fn browser_args(&self, mode: LaunchMode, port: u16, profile: &Path) -> Vec<String> {
        let mut args: Vec<String> = AUTOMATION_ARGS.iter().map(|s| s.to_string()).collect();
        if mode == LaunchMode::Headless {
            // chromiumoxide's headless defaults, kept for parity.
            args.extend(["--headless", "--hide-scrollbars", "--mute-audio"].map(str::to_owned));
        } else if self.app_window {
            // The headed window is a chromeless app window: no address
            // bar, tab strip, or bookmarks — the pure surface agents
            // operate and humans watch.
            args.push("--app=about:blank".to_owned());
        }
        args.push(format!("--remote-debugging-port={port}"));
        if !self
            .extra_args
            .iter()
            .any(|argument| argument.starts_with("--user-data-dir"))
        {
            args.push(format!("--user-data-dir={}", profile.display()));
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Reads a user-supplied debugging port from the extras, so an
    /// override still lands on the port rutter waits for.
    fn configured_port(&self) -> Option<u16> {
        self.extra_args.iter().find_map(|argument| {
            argument
                .strip_prefix("--remote-debugging-port=")
                .and_then(|value| value.parse().ok())
        })
    }

    /// Spawns the browser process. Streams are severed: rutter discovers
    /// the endpoint by polling the port, and unread pipes would deadlock
    /// a chatty browser. The port and profile also travel as environment
    /// variables for shells that rebuild their command line and drop
    /// Chromium switches — Electron does exactly that — while plain
    /// browsers ignore the extra environment.
    fn spawn_browser(
        &self,
        args: &[String],
        port: u16,
        profile: &Path,
    ) -> Result<Child, EngineError> {
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .args(args)
            .env("RUTTER_CDP_PORT", port.to_string())
            .env("RUTTER_PROFILE", profile.as_os_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            // Never allocate a console for the child.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command.spawn().map_err(|error| EngineError::LaunchFailed {
            detail: format!("spawn {}: {error}", self.executable.display()),
        })
    }

    /// Reconnects until the debugging endpoint answers or the launch
    /// budget runs out. Each attempt is bounded, so an endpoint that
    /// accepts but never answers cannot stall the loop.
    async fn connect_with_retries(&self, port: u16) -> Result<Browser, EngineError> {
        let url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        loop {
            let attempt =
                tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, Browser::connect(url.clone())).await;
            if let Ok(Ok((browser, mut handler))) = attempt {
                // The handler stream must be drained for any CDP traffic
                // to flow; it ends when the connection closes.
                tokio::spawn(async move { while handler.next().await.is_some() {} });
                return Ok(browser);
            }
            if Instant::now() >= deadline {
                return Err(EngineError::LaunchFailed {
                    detail: format!(
                        "browser did not open its debugging endpoint on port {port} within the launch budget"
                    ),
                });
            }
            tokio::time::sleep(CONNECT_RETRY_DELAY).await;
        }
    }
}

/// Picks a free loopback port by binding and releasing a listener. The
/// released port can race with other processes; the launch then fails
/// its connect budget and the supervisor retries with a fresh one.
fn pick_debug_port() -> Result<u16, EngineError> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
        EngineError::LaunchFailed {
            detail: format!("reserve a debugging port: {error}"),
        }
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| EngineError::LaunchFailed {
            detail: format!("reserve a debugging port: {error}"),
        })?
        .port();
    Ok(port)
}

/// Per-launch profile directory under the OS temp dir: a fresh browser
/// carries no bookmarks, history, or login state, and two engines never
/// share a profile. The browser creates the directory; the OS temp
/// cleaner reclaims it after use.
fn profile_dir() -> PathBuf {
    static LAUNCH: AtomicU64 = AtomicU64::new(0);
    let serial = LAUNCH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("rutter-engine-{}-{}", std::process::id(), serial))
}

#[async_trait]
impl EngineLauncher for CdpLauncher {
    fn describe(&self) -> String {
        format!("cdp engine at {}", self.executable.display())
    }

    async fn launch(&self, mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let profile = profile_dir();
        let port = match self.configured_port() {
            Some(port) => port,
            None => pick_debug_port()?,
        };
        let args = self.browser_args(mode, port, &profile);
        let child = self.spawn_browser(&args, port, &profile)?;
        // Every error path below drops the guard, killing the child, so
        // the policy's next attempt starts clean.
        let process = BrowserProcess::new(child);

        let browser = self.connect_with_retries(port).await?;

        // The version query carries a deadline like every CDP call: it
        // runs inside the supervisor's restart loop, so a browser that
        // accepts the socket but never answers would otherwise park the
        // heartbeat (and its restart mutex) forever, disabling all
        // recovery. On timeout the dropped process guard kills the
        // child.
        let version =
            error::with_deadline("engine_version", error::COMMAND_TIMEOUT, browser.version())
                .await?;
        Ok(Arc::new(CdpEngine::new(
            Arc::new(tokio::sync::Mutex::new(browser)),
            self.backend.clone(),
            version.product,
            mode,
            process,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rutter_engine::descriptor::EngineBackend;

    fn launcher(extra: &[&str]) -> CdpLauncher {
        CdpLauncher::new(PathBuf::from("browser.exe"), EngineBackend::Chromium)
            .with_extra_args(extra.iter().map(|s| s.to_string()).collect())
    }

    fn find<'a>(args: &'a [String], prefix: &str) -> Option<&'a str> {
        args.iter()
            .find_map(|argument| argument.strip_prefix(prefix))
    }

    #[test]
    fn headless_matches_the_automation_defaults() {
        let args = launcher(&[]).browser_args(LaunchMode::Headless, 9222, Path::new("p"));
        assert_eq!(find(&args, "--remote-debugging-port="), Some("9222"));
        assert_eq!(find(&args, "--user-data-dir="), Some("p"));
        for flag in ["--headless", "--hide-scrollbars", "--mute-audio"] {
            assert!(args.contains(&flag.to_owned()), "missing {flag}");
        }
        assert!(args.contains(&"--enable-automation".to_owned()));
        assert!(!args.contains(&"--app=about:blank".to_owned()));
    }

    #[test]
    fn headed_is_a_chromeless_app_window_when_asked() {
        let args = launcher(&[]).with_app_window(true).browser_args(
            LaunchMode::Headed,
            9222,
            Path::new("p"),
        );
        assert!(args.contains(&"--app=about:blank".to_owned()));
        assert!(!args.contains(&"--headless".to_owned()));
    }

    #[test]
    fn headed_plain_window_when_app_mode_not_requested() {
        let args = launcher(&[]).browser_args(LaunchMode::Headed, 9222, Path::new("p"));
        assert!(!args.contains(&"--app=about:blank".to_owned()));
        assert!(!args.contains(&"--headless".to_owned()));
    }

    #[test]
    fn explicit_profile_overrides_the_per_launch_one() {
        let args = launcher(&["--user-data-dir=keep"]).browser_args(
            LaunchMode::Headless,
            9222,
            Path::new("generated"),
        );
        assert_eq!(find(&args, "--user-data-dir="), Some("keep"));
    }

    #[test]
    fn configured_port_is_read_from_the_extras() {
        assert_eq!(
            launcher(&["--remote-debugging-port=9333"]).configured_port(),
            Some(9333)
        );
        assert_eq!(launcher(&[]).configured_port(), None);
    }
}
