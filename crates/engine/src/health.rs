//! Results of engine health probes.

/// Outcome of one supervised health probe.
///
/// The supervisor probes on a heartbeat cadence; a failed report is a
/// trigger for restart logic, never a crash of the orchestration layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// Whether the engine answered the probe correctly.
    pub healthy: bool,
    /// Backend version string observed during the probe, when the engine
    /// was reachable enough to answer.
    pub backend_version: Option<String>,
    /// Human-readable detail about a failing probe, when unhealthy.
    pub detail: Option<String>,
}
