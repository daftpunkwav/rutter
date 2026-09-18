//! Session and auto-wait configuration.
//!
//! Boundary: numeric policy for one process. The values are defaults
//! pinned by `docs/TOOL_SPEC.md` §3; a TOML config file arrives with
//! the policy rules in M2.

use std::time::Duration;

use rutter_engine::config::ContextConfig;

/// Defaults for one server process (blueprint §7.8 tool semantics).
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Maximum pages per session context.
    pub max_pages: usize,
    /// Maximum concurrent sessions this server hosts. Each session
    /// holds a browser context, so the cap bounds the engine's memory
    /// and target count against runaway clients.
    pub max_sessions: usize,
    /// Deadline for one navigation.
    pub navigation_timeout: Duration,
    /// Budget of each auto-wait phase (visible, stable, enabled).
    pub phase_timeout: Duration,
    /// Interval between the two stability samples.
    pub stability_sample_interval: Duration,
    /// Wait after an action before taking the fresh snapshot.
    pub settle: Duration,
    /// Default budget for `wait_for` when the caller sends none.
    pub wait_for_budget: Duration,
    /// Poll interval for auto-wait phases and `wait_for`.
    pub poll_interval: Duration,
    /// Window a human has to answer an approval request
    /// (blueprint §7.6: default 120 s).
    pub approval_timeout: Duration,
}

impl SessionConfig {
    /// Context caps handed to the engine.
    pub fn context_config(&self) -> ContextConfig {
        ContextConfig {
            max_pages: self.max_pages,
            navigation_timeout: self.navigation_timeout,
            screenshot_min_interval: Duration::from_millis(500),
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_pages: 8,
            max_sessions: 8,
            navigation_timeout: Duration::from_secs(30),
            phase_timeout: Duration::from_secs(5),
            stability_sample_interval: Duration::from_millis(80),
            settle: Duration::from_millis(250),
            wait_for_budget: Duration::from_secs(10),
            poll_interval: Duration::from_millis(100),
            approval_timeout: Duration::from_secs(120),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_tool_spec() {
        let config = SessionConfig::default();
        assert_eq!(config.max_pages, 8);
        assert_eq!(config.max_sessions, 8);
        assert_eq!(config.phase_timeout, Duration::from_secs(5));
        assert_eq!(config.settle, Duration::from_millis(250));
        assert_eq!(config.wait_for_budget, Duration::from_secs(10));
        assert_eq!(config.context_config().max_pages, 8);
    }
}
