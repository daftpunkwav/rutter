//! Supervisor lifecycle tests over a scripted launcher and engine:
//! heartbeat replacement, backoff, breaker, failed-start recovery —
//! all through the public `Supervisor` API.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::context::ContextHandle;
use rutter_engine::descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::health::HealthReport;
use rutter_engine::supervisor::{EngineLauncher, RestartPolicy, Supervisor};

/// Scriptable engine: starts healthy, dies on demand; after
/// [`MockEngine::die`] every health probe fails.
struct MockEngine {
    alive: AtomicBool,
    serial: usize,
}

impl MockEngine {
    fn die(&self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl Engine for MockEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            backend: EngineBackend::ChromiumHeadlessShell,
            version: format!("mock-{}", self.serial),
            capabilities: EngineCapabilities {
                headless: true,
                headed: false,
                screencast: false,
                per_context_isolation: false,
            },
        }
    }

    async fn create_context(
        &self,
        _config: ContextConfig,
    ) -> Result<Arc<dyn ContextHandle>, EngineError> {
        Err(EngineError::Unsupported {
            operation: "create_context".to_owned(),
            reason: "mock engine hosts no contexts".to_owned(),
        })
    }

    async fn health(&self) -> Result<HealthReport, EngineError> {
        if self.alive.load(Ordering::SeqCst) {
            Ok(HealthReport {
                healthy: true,
                backend_version: Some(format!("mock-{}", self.serial)),
                detail: None,
            })
        } else {
            Err(EngineError::Terminated)
        }
    }

    async fn shutdown(&self) -> Result<(), EngineError> {
        self.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// Launcher producing mock engines and keeping typed handles so
/// tests can kill them without downcasts.
struct MockLauncher {
    launches: AtomicUsize,
    launched: Mutex<Vec<Arc<MockEngine>>>,
}

impl MockLauncher {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            launches: AtomicUsize::new(0),
            launched: Mutex::new(Vec::new()),
        })
    }

    fn launch_count(&self) -> usize {
        self.launches.load(Ordering::SeqCst)
    }

    fn engine(&self, serial: usize) -> Arc<MockEngine> {
        self.launched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(serial)
            .cloned()
            .expect("engine with serial")
    }
}

#[async_trait]
impl EngineLauncher for MockLauncher {
    fn describe(&self) -> String {
        "mock".to_owned()
    }

    async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let serial = self.launches.fetch_add(1, Ordering::SeqCst);
        let engine = Arc::new(MockEngine {
            alive: AtomicBool::new(true),
            serial,
        });
        self.launched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Arc::clone(&engine));
        Ok(engine)
    }
}

fn fast_supervisor(launcher: Arc<dyn EngineLauncher>) -> Supervisor {
    Supervisor::new(launcher, LaunchMode::Headless)
        .with_heartbeat(Duration::from_millis(10))
        .with_policy(RestartPolicy::with_backoff(
            5,
            Duration::from_millis(500),
            Duration::from_millis(1),
            Duration::from_millis(5),
        ))
}

/// Polls until the supervisor reports an engine whose version
/// differs from `previous`, or times out. `Terminated` while the
/// swap is in flight is part of the contract, so it is retried.
async fn wait_for_replacement(supervisor: &Supervisor, previous: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match supervisor.engine().await {
            Ok(engine) => {
                let version = engine.descriptor().version;
                if version != previous {
                    return version;
                }
            }
            Err(EngineError::Terminated) => {}
            Err(error) => panic!("unexpected engine error during restart: {error}"),
        }
        if Instant::now() > deadline {
            panic!("engine was not replaced within 5 s");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn start_provides_engine_and_shutdown_stops_it() {
    let launcher = MockLauncher::new();
    let supervisor = fast_supervisor(Arc::clone(&launcher) as Arc<dyn EngineLauncher>);

    supervisor.start().await.expect("start");
    let engine = supervisor.engine().await.expect("engine");
    assert_eq!(engine.descriptor().version, "mock-0");

    supervisor.shutdown().await;
    assert!(matches!(
        supervisor.engine().await,
        Err(EngineError::Terminated)
    ));
    assert_eq!(launcher.launch_count(), 1);
}

#[tokio::test]
async fn heartbeat_replaces_dead_engine() {
    let launcher = MockLauncher::new();
    let supervisor = fast_supervisor(Arc::clone(&launcher) as Arc<dyn EngineLauncher>);
    supervisor.start().await.expect("start");
    assert_eq!(
        supervisor
            .engine()
            .await
            .expect("engine")
            .descriptor()
            .version,
        "mock-0"
    );

    launcher.engine(0).die();
    let version = wait_for_replacement(&supervisor, "mock-0").await;
    assert_eq!(version, "mock-1");
    assert!(launcher.launch_count() >= 2);
}

#[tokio::test]
async fn breaker_open_fails_engine_calls_with_terminated() {
    struct FailingLauncher;
    #[async_trait]
    impl EngineLauncher for FailingLauncher {
        fn describe(&self) -> String {
            "failing".to_owned()
        }
        async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
            Err(EngineError::LaunchFailed {
                detail: "no browser on this machine".to_owned(),
            })
        }
    }

    let supervisor = Supervisor::new(Arc::new(FailingLauncher), LaunchMode::Headless)
        .with_heartbeat(Duration::from_millis(5))
        .with_policy(RestartPolicy::with_backoff(
            2,
            Duration::from_millis(100),
            Duration::from_millis(1),
            Duration::from_millis(1),
        ));
    // The initial start exhausts the window; the breaker opens and
    // every engine() call reports Terminated.
    assert!(matches!(
        supervisor.start().await,
        Err(EngineError::Terminated)
    ));
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(matches!(
        supervisor.engine().await,
        Err(EngineError::Terminated)
    ));
}

/// Launcher that fails its first launches, then recovers.
struct FlakyLauncher {
    failures_left: AtomicUsize,
    launches: AtomicUsize,
}

#[async_trait]
impl EngineLauncher for FlakyLauncher {
    fn describe(&self) -> String {
        "flaky".to_owned()
    }
    async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let serial = self.launches.fetch_add(1, Ordering::SeqCst);
        if self.failures_left.fetch_sub(1, Ordering::SeqCst) > 1 {
            return Err(EngineError::LaunchFailed {
                detail: "transient failure".to_owned(),
            });
        }
        Ok(Arc::new(MockEngine {
            alive: AtomicBool::new(true),
            serial,
        }))
    }
}

#[tokio::test]
async fn supervisor_recovers_after_a_failed_start() {
    let supervisor = Supervisor::new(
        Arc::new(FlakyLauncher {
            failures_left: AtomicUsize::new(3),
            launches: AtomicUsize::new(0),
        }),
        LaunchMode::Headless,
    )
    .with_heartbeat(Duration::from_millis(5))
    .with_policy(RestartPolicy::with_backoff(
        2,
        Duration::from_millis(100),
        Duration::from_millis(1),
        Duration::from_millis(1),
    ));

    // The initial start exhausts its window and reports failure, but
    // the heartbeat keeps retrying and the engine comes back.
    assert!(matches!(
        supervisor.start().await,
        Err(EngineError::Terminated)
    ));

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match supervisor.engine().await {
            Ok(engine) => {
                assert_eq!(engine.descriptor().version, "mock-2");
                break;
            }
            Err(EngineError::Terminated) => {}
            Err(error) => panic!("unexpected engine error: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "supervisor did not recover within 5 s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    supervisor.shutdown().await;
}
