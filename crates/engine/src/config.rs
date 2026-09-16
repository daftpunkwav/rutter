//! Launch and context configuration for engines.

use std::time::Duration;

/// Visibility mode of an engine launch.
///
/// The mode is engine-level configuration and does not change any API
/// above the [`crate::Engine`] trait: `Headless` is the default in serve
/// mode, `Headed` is used by browse mode and interactive debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// No visible window; the default for supervised operation.
    Headless,
    /// A visible engine window operated by a human.
    Headed,
}

/// Resource caps applied to one context.
///
/// Caps protect the shared engine process from a single session; they are
/// enforced by the engine implementation, not by callers.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextConfig {
    /// Maximum number of pages (tabs) the context may hold.
    pub max_pages: usize,
    /// Deadline for a single navigation.
    pub navigation_timeout: Duration,
    /// Minimum interval between two screenshots; caps the capture rate.
    pub screenshot_min_interval: Duration,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_pages: 8,
            navigation_timeout: Duration::from_secs(30),
            screenshot_min_interval: Duration::from_millis(500),
        }
    }
}
