//! Descriptions of engine backends and their capabilities.

use std::fmt;

/// Which browser backend an engine runs.
///
/// New backends (any CDP-compatible engine) are added behind the same
/// trait; the list grows through the registration point in the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineBackend {
    /// The Chrome for Testing headless shell, the initial backend.
    ChromiumHeadlessShell,
    /// The full Chrome for Testing browser; required for headed windows.
    Chromium,
}

impl fmt::Display for EngineBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Kebab-case names as reported to event consumers
        // (`Event::EngineStarted`): `chrome-headless-shell`.
        let name = match self {
            Self::ChromiumHeadlessShell => "chrome-headless-shell",
            Self::Chromium => "chrome",
        };
        f.write_str(name)
    }
}

/// What an engine backend reports about itself.
///
/// This is self-reported metadata: it reaches diagnostics and the
/// `EngineStarted` event. It is not yet a behavior gate — no code path
/// consults a capability before attempting the operation. A backend
/// refusal surfaces as an [`crate::error::EngineError`] from the call
/// that hit it; where a degraded mode preserves function, the backend
/// degrades and reports the reduced capability here instead (see
/// [`EngineCapabilities::per_context_isolation`]). A second engine is
/// the moment to decide which of these become checked; until then the
/// fields say what the one backend is, not what callers may assume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCapabilities {
    /// Whether the backend can run headless.
    pub headless: bool,
    /// Whether the backend can open a visible window.
    pub headed: bool,
    /// Whether the backend supports CDP screencast streaming.
    pub screencast: bool,
    /// Whether contexts get browser-level isolation into separate cookie
    /// and storage units. Effective state, not a static trait: a backend
    /// that discovers at runtime it cannot create isolated contexts
    /// reports `false` from then on, so read it from a fresh
    /// `descriptor()` at the moment of decision rather than caching it.
    pub per_context_isolation: bool,
}

/// Identifying and capability information about a running engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDescriptor {
    /// Which backend the engine runs.
    pub backend: EngineBackend,
    /// Backend version string, as reported by the engine process.
    pub version: String,
    /// Capabilities supported by this engine.
    pub capabilities: EngineCapabilities,
}
