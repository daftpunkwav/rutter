//! Static descriptions of engine backends and their capabilities.

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

/// What an engine backend can do; consumers must check before relying on
/// an optional capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCapabilities {
    /// Whether the backend can run headless.
    pub headless: bool,
    /// Whether the backend can open a visible window.
    pub headed: bool,
    /// Whether the backend supports CDP screencast streaming.
    pub screencast: bool,
    /// Whether the backend isolates contexts into separate cookie and
    /// storage units.
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
