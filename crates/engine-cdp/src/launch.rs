//! Launcher producing CDP engines from a resolved browser executable.
//!
//! Boundary: the CDP registration point. A launcher pairs an executable
//! path with its backend identity (headless shell or full Chrome) and
//! produces engine instances; the binary decides which launcher to
//! build. Process supervision is the supervisor's concern, not this
//! factory's.
//!
//! rutter spawns the browser process itself and attaches via the
//! debugging port instead of letting chromiumoxide spawn it, for a
//! Windows fact the library's spawner cannot work around:
//!
//! - Launcher-style executables (Edge) relay to a child process and
//!   exit immediately, closing the stderr stream chromiumoxide parses
//!   the websocket address from. A loopback port polled through
//!   `json/version` is indifferent to who survives the spawn.
//!
//! The engine owns the spawned child so a dropped engine takes the
//! browser with it; graceful shutdown goes through the CDP
//! `Browser.close` command (see [`CdpEngine::shutdown`]), which also
//! reaches a browser whose intermediate launcher already exited. An
//! abnormal rutter kill can therefore leak a browser whose spawn
//! exited — the known cost of launcher-style executables.

use std::io::{Read, Seek, SeekFrom};
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
        // An override names its own port, and Chromium takes the last
        // spelling it sees: adding ours beside it would leave the two
        // flags disagreeing about which endpoint to wait for.
        if self.configured_port().is_none() {
            args.push(format!("--remote-debugging-port={port}"));
        }
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
    /// override still lands on the port rutter waits for. Chromium
    /// accepts both `--remote-debugging-port=9333` and the two-argument
    /// `--remote-debugging-port 9333`; reading only the first spelling
    /// made the second a silent mismatch — rutter waited on a port
    /// nobody opened until the whole launch budget ran out.
    fn configured_port(&self) -> Option<u16> {
        let mut arguments = self.extra_args.iter();
        while let Some(argument) = arguments.next() {
            if let Some(port) = argument
                .strip_prefix("--remote-debugging-port=")
                .and_then(|value| value.parse().ok())
            {
                return Some(port);
            }
            if argument == "--remote-debugging-port"
                && let Some(port) = arguments.next().and_then(|value| value.parse().ok())
            {
                return Some(port);
            }
        }
        None
    }

    /// Spawns the browser process. Stdin and stdout are severed; stderr
    /// goes to a launch log file — an unread pipe would deadlock a
    /// chatty browser, while a file the OS writes cannot block the
    /// child, and a launch that never opens its endpoint gets its dying
    /// words surfaced in the error. The port and profile also travel as
    /// environment variables for shells that rebuild their command line
    /// and drop Chromium switches — Electron does exactly that — while
    /// plain browsers ignore the extra environment.
    fn spawn_browser(
        &self,
        args: &[String],
        port: u16,
        profile: &Path,
        stderr: Stdio,
    ) -> Result<Child, EngineError> {
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .args(args)
            .env("RUTTER_CDP_PORT", port.to_string())
            .env("RUTTER_PROFILE", profile.as_os_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
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
    /// accepts but never answers cannot stall the loop. On failure the
    /// engine's stderr log is surfaced: a browser that never opens the
    /// endpoint almost always says why first (a missing library, a
    /// blocked sandbox), and without this the reason dies with the
    /// null'd stream it wrote to.
    async fn connect_with_retries(
        &self,
        port: u16,
        stderr_log: &Path,
    ) -> Result<Browser, EngineError> {
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
                let stderr = stderr_tail(stderr_log);
                let detail = format!(
                    "browser did not open its debugging endpoint on port {port} within the launch budget"
                );
                return Err(EngineError::LaunchFailed {
                    detail: if stderr.is_empty() {
                        detail
                    } else {
                        format!("{detail}; engine stderr: {stderr}")
                    },
                });
            }
            tokio::time::sleep(CONNECT_RETRY_DELAY).await;
        }
    }
}

/// The engine's last words from its launch stderr log, whitespace-
/// collapsed into a single line. A missing, empty, or non-UTF-8 log
/// (the file can also be lost to a wiped temp dir) yields an empty
/// string; the budget message alone still tells the caller what
/// happened, just not why.
fn stderr_tail(log: &Path) -> String {
    const MAX_TAIL_BYTES: u64 = 4096;
    let Ok(mut file) = std::fs::File::open(log) else {
        return String::new();
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let start = len.saturating_sub(MAX_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
/// share a profile. The name carries a random component, because pid and
/// serial alone are guessable by another local user, who could otherwise
/// pre-create the path (a launch-killing squat) or plant a symlink at
/// it; the directory is created here, exclusively, so nothing the
/// browser later touches was ever a stranger-planted name.
///
/// The directory is owner-only on Unix. It holds the browser's cookies,
/// history, and login state, and a directory made with the process
/// umask would be readable — and, under a permissive umask, writable —
/// by every other local user on the machine. Requesting the mode
/// outright means the umask can only clear bits from it, never add
/// them.
fn profile_dir() -> Result<PathBuf, EngineError> {
    static LAUNCH: AtomicU64 = AtomicU64::new(0);
    let serial = LAUNCH.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rutter-engine-{}-{}-{:016x}",
        std::process::id(),
        serial,
        launch_secret()
    ));
    // Owner-only on Unix, where the mode is the whole point; elsewhere
    // the per-user ACL of the temp directory is what bounds access, and
    // this is the same exclusive create.
    //
    // The create's mode is masked by the process umask, and the umask can
    // clear the owner bits too: a launcher running under a umask of 0777
    // would otherwise hand the browser a profile it cannot read. The mode
    // is therefore set outright once the directory exists, and failing to
    // set it is the same refusal as failing to create it -- with the
    // directory removed first, since the caller gets no path back and a
    // refusal that repeated would otherwise leave one behind each time.
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt;
        use std::os::unix::fs::PermissionsExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .and_then(|()| {
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).inspect_err(
                    |_| {
                        let _ = std::fs::remove_dir(&dir);
                    },
                )
            })
    };
    #[cfg(not(unix))]
    let created = std::fs::create_dir(&dir);
    match created {
        Ok(()) => Ok(dir),
        // A collision on a freshly random name is not retried: it means
        // something on this machine is deliberately racing this launch,
        // and that is a refusal, not bad luck to route around.
        Err(error) => Err(EngineError::LaunchFailed {
            detail: format!("create engine profile directory {}: {error}", dir.display()),
        }),
    }
}

/// 64 bits keyed by OS entropy, the same construction the dashboard
/// token uses (crates/dashboard/src/lib.rs `generate_token`): kept
/// inline on purpose — four lines each, in layers that share no crate —
/// and cross-referenced both ways. Unpredictable to another process,
/// unlike the pid and the per-process serial the directory name also
/// carries.
fn launch_secret() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hash, Hasher};
    let mut hasher = RandomState::new().build_hasher();
    std::time::SystemTime::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    hasher.finish()
}

#[async_trait]
impl EngineLauncher for CdpLauncher {
    fn describe(&self) -> String {
        format!("cdp engine at {}", self.executable.display())
    }

    async fn launch(&self, mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let profile = profile_dir()?;
        // The engine's stderr lands inside the profile directory this
        // launch just created exclusively — a sibling path in the shared
        // temp dir would be one another local user could have planted a
        // symlink at, and a create that follows a symlink writes through
        // it. An unread pipe would deadlock a chatty browser, while a
        // file the OS writes cannot block the child, and a launch that
        // never opens its endpoint gets its dying words surfaced in the
        // error. The OS temp cleaner reclaims the directory like the
        // profile itself.
        let stderr_log = profile.join("launch-stderr.log");
        let port = match self.configured_port() {
            Some(port) => port,
            None => pick_debug_port()?,
        };
        let args = self.browser_args(mode, port, &profile);
        let stderr = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stderr_log)
            .map_err(|error| EngineError::LaunchFailed {
                detail: format!("create launch stderr log {}: {error}", stderr_log.display()),
            })?;
        let child = self.spawn_browser(&args, port, &profile, stderr.into())?;
        // Every error path below drops the guard, killing the child, so
        // the policy's next attempt starts clean.
        let process = BrowserProcess::new(child);

        let browser = self.connect_with_retries(port, &stderr_log).await?;

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
        // Chromium also accepts the two-argument spelling; reading only
        // the `=` form left rutter waiting on a port the browser never
        // opened.
        assert_eq!(
            launcher(&["--remote-debugging-port", "9333"]).configured_port(),
            Some(9333)
        );
        assert_eq!(
            launcher(&["--no-sandbox", "--remote-debugging-port", "9333"]).configured_port(),
            Some(9333),
            "the value is read from the argument that follows the flag"
        );
        assert_eq!(
            launcher(&["--remote-debugging-port"]).configured_port(),
            None,
            "a flag with no value names no port"
        );
        assert_eq!(launcher(&[]).configured_port(), None);
    }

    #[test]
    fn an_overridden_port_is_never_passed_twice() {
        // Chromium takes the last spelling it sees, so rutter's own flag
        // next to the override could leave the endpoint it waits for and
        // the one the browser opens disagreeing.
        for extra in [
            vec!["--remote-debugging-port=9333"],
            vec!["--remote-debugging-port", "9333"],
        ] {
            let args = launcher(&extra).browser_args(LaunchMode::Headless, 9222, Path::new("p"));
            let flags = args
                .iter()
                .filter(|argument| argument.starts_with("--remote-debugging-port"))
                .count();
            assert_eq!(flags, 1, "one port flag for {extra:?}: {args:?}");
            assert!(
                !args.iter().any(|argument| argument.contains("9222")),
                "rutter's own port never joins an override: {args:?}"
            );
            assert!(
                args.iter().any(|argument| argument.contains("9333")),
                "the override reaches the command line: {args:?}"
            );
        }
    }

    #[test]
    fn a_free_launch_still_passes_its_own_port() {
        let args =
            launcher(&["--no-sandbox"]).browser_args(LaunchMode::Headless, 9222, Path::new("p"));
        assert_eq!(find(&args, "--remote-debugging-port="), Some("9222"));
    }

    #[test]
    fn stderr_tail_is_empty_for_a_missing_log() {
        assert_eq!(stderr_tail(Path::new("no-such-launch-log.tmp")), "");
    }

    #[test]
    fn a_profile_directory_is_created_and_never_reused() {
        let first = profile_dir().expect("profile dir");
        assert!(first.is_dir(), "the launch owns its profile from here on");
        let second = profile_dir().expect("second profile dir");
        assert_ne!(
            first, second,
            "two launches never share a profile directory"
        );
        let name = first
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(&format!("rutter-engine-{}-", std::process::id())),
            "the name stays recognizable for cleanup tooling: {name}"
        );
        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    /// Serializes every fixture in this binary that touches the
    /// filesystem against the restrictive-umask one: the umask is
    /// process-wide, and a file or directory created inside its window
    /// comes out with the owner bits masked away -- a tempfile drops
    /// from `0600` to `0400`, which its later `fs::write` cannot reopen.
    /// The umask half is Unix-only; the lock exists everywhere so the
    /// fixtures that take it need no gate of their own.
    static FILESYSTEM_FIXTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    #[test]
    fn a_profile_directory_is_owner_only() {
        // The profile holds the browser's cookies, history, and login
        // state, so another local user must not be able to read it --
        // and must not be able to create files inside it, which is what
        // makes planting a symlink at a path the browser writes to
        // impossible.
        let _fixture = FILESYSTEM_FIXTURE.lock().unwrap_or_else(|p| p.into_inner());
        assert_owner_only("an ordinary umask");
    }

    /// Pins the mode under a umask that would break it.
    ///
    /// The create's own mode is masked by the umask, so under a
    /// restrictive one the directory comes out without the owner bits a
    /// browser needs to read its own profile -- which is exactly the
    /// regression the explicit permissions after the create exist to
    /// prevent, and exactly what an ordinary umask cannot expose: there
    /// the masked mode still lands on `0700` with the set removed. The
    /// umask is process-wide, so the fixture holds
    /// [`FILESYSTEM_FIXTURE`] for its whole window and the guard puts
    /// the old mask back even when the assertion fails.
    #[cfg(unix)]
    #[test]
    fn a_profile_directory_survives_a_restrictive_umask() {
        let _fixture = FILESYSTEM_FIXTURE.lock().unwrap_or_else(|p| p.into_inner());
        let _umask = UmaskGuard(set_umask(0o277));
        assert_owner_only("a restrictive umask");
    }

    /// Restores the umask the fixture replaced, whatever happened
    /// inside the window.
    #[cfg(unix)]
    struct UmaskGuard(u32);

    #[cfg(unix)]
    impl Drop for UmaskGuard {
        fn drop(&mut self) {
            set_umask(self.0);
        }
    }

    /// Sets the process umask and returns the one it replaced.
    ///
    /// `umask` is a plain syscall with no failure mode; the only reason
    /// for the escape hatch is that the workspace denies `unsafe_code`
    /// outright, and the call is the one way a test can control what
    /// the create's mode is masked by.
    #[cfg(unix)]
    #[allow(unsafe_code)]
    fn set_umask(mask: u32) -> u32 {
        // SAFETY: `umask` only sets the process file-mode creation mask
        // and returns the previous one; it touches no memory and cannot
        // fail.
        let previous = unsafe { libc::umask(mask as libc::mode_t) }; // nosemgrep
        previous as u32
    }

    /// Runs [`profile_dir`] under the caller's umask and asserts the
    /// directory came out owner-only.
    #[cfg(unix)]
    fn assert_owner_only(under: &str) {
        use std::os::unix::fs::PermissionsExt;
        let profile = profile_dir().expect("profile dir");
        let mode = std::fs::metadata(&profile)
            .expect("metadata")
            .permissions()
            .mode();
        let _ = std::fs::remove_dir_all(&profile);
        assert_eq!(
            mode & 0o777,
            0o700,
            "the profile stays owner-only under {under}"
        );
    }

    #[test]
    fn stderr_tail_collapses_the_log_into_one_line() {
        // A scratch file of the harness's own name: a fixed path under
        // the shared temp dir is one another local user could have
        // planted a symlink at, and the sibling tail test would collide
        // with it while running concurrently. The shared fixture lock
        // keeps the scratch file out of the restrictive-umask window:
        // created there, its 0600 comes out 0400 and the write below
        // could not reopen it.
        let _fixture = FILESYSTEM_FIXTURE.lock().unwrap_or_else(|p| p.into_inner());
        let log = tempfile::Builder::new()
            .prefix("rutter-tail-collapse-")
            .suffix(".log")
            .tempfile()
            .expect("scratch log");
        std::fs::write(
            log.path(),
            "first line\n\nerror while loading\n  shared  libraries\n",
        )
        .unwrap();
        let tail = stderr_tail(log.path());
        assert_eq!(tail, "first line error while loading shared libraries");
    }

    #[test]
    fn stderr_tail_keeps_only_the_end_of_a_chatty_log() {
        let log = tempfile::Builder::new()
            .prefix("rutter-tail-bounded-")
            .suffix(".log")
            .tempfile()
            .expect("scratch log");
        let noise = "x".repeat(100_000);
        std::fs::write(log.path(), format!("{noise}\nTHE ACTUAL CAUSE")).unwrap();
        let tail = stderr_tail(log.path());
        assert!(tail.ends_with("THE ACTUAL CAUSE"));
        assert!(tail.len() < 5_000, "the tail must stay bounded");
    }
}
