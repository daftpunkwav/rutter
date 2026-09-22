//! The only engine abstraction in the codebase.
//!
//! Boundary: engine-level lifecycle. Swapping or adding engines is
//! confined to implementing this trait plus a registration point in the
//! CLI; nothing above this trait may know which backend is running.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::ContextConfig;
use crate::context::ContextHandle;
use crate::descriptor::EngineDescriptor;
use crate::error::EngineError;
use crate::health::HealthReport;

/// A supervised browser engine managed by rutter.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Identifying and capability information about this engine.
    fn descriptor(&self) -> EngineDescriptor;

    /// Creates a new context with the given resource caps.
    ///
    /// Isolation is best-effort: a backend that cannot isolate contexts
    /// returns one sharing the engine default storage, and reports that
    /// through `descriptor().capabilities.per_context_isolation`.
    async fn create_context(
        &self,
        config: ContextConfig,
    ) -> Result<Arc<dyn ContextHandle>, EngineError>;

    /// Probes engine health; used by the supervisor's heartbeat.
    async fn health(&self) -> Result<HealthReport, EngineError>;

    /// Shuts the engine down gracefully; further calls fail with
    /// [`EngineError::Terminated`].
    async fn shutdown(&self) -> Result<(), EngineError>;
}
