//! The approval audit trail: one JSON line per supervised decision.
//!
//! Responsibility: persist what a human was asked and how it was answered,
//! so a supervision question can be answered after the process is gone.
//!
//! Boundary: append-only writes, no reads. The in-memory event backbone is
//! the live view (docs/events.md); this is the record that survives it, and
//! a session close deliberately does not erase it the way it erases the
//! session's replay ring.
//!
//! Why the session layer owns this: the audit has to exist whether or not
//! anyone is watching the dashboard. A record written only by the dashboard
//! would be missing exactly when a decision timed out with no viewer
//! connected — the case supervision most needs to explain.
//!
//! Limits worth stating: the entry records the decision's outcome and the
//! brief it was made from, not a verified human identity. On one machine and
//! one user account there is no way for rutter to tell a person at a
//! dashboard from a process that holds the dashboard token
//! (docs/dashboard.md §2).

use std::path::{Path, PathBuf};

use rutter_core::ids::{PageId, SessionId};
use rutter_policy::{ApprovalBrief, VerdictBasis};
use serde::Serialize;

/// How a parked operation ended. The session layer owns this label because
/// it knows about endings the broker's three outcomes do not cover: an MCP
/// client that disconnected mid-park cancelled the wait rather than letting
/// anyone time out on it.
pub(crate) const GRANTED: &str = "granted";
/// A human refused the operation.
pub(crate) const DENIED: &str = "denied";
/// The answer window closed with nobody replying.
pub(crate) const TIMED_OUT: &str = "timed_out";
/// The waiting operation went away with its client; no human answered.
pub(crate) const CANCELLED: &str = "cancelled";

/// One supervised decision, as it goes to disk.
#[derive(Serialize)]
struct AuditEntry {
    /// RFC 3339 UTC time the decision was recorded.
    at: String,
    /// Session the operation belonged to.
    session: String,
    /// Page the operation targeted.
    page: String,
    /// Broker-minted identifier the decision answered.
    request_id: String,
    /// How the operation was resolved; one of the labels above.
    outcome: &'static str,
    /// Milliseconds the operation waited for an answer.
    waited_ms: u128,
    /// The class policy judged.
    class: String,
    /// The canonical URL judged; `null` when there was none.
    judged_url: Option<String>,
    /// Which part of the rule set asked.
    basis: String,
    /// What a grant would have authorized.
    effect: String,
}

/// Appends approval records to one file. `None` for the path disables the
/// trail, which is what a session without a state directory gets.
pub(crate) struct ApprovalAudit {
    path: Option<PathBuf>,
}

impl ApprovalAudit {
    /// An audit writing to `path`, or a no-op one when `path` is `None`.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// Derives the audit file from a session's storage path: the sibling
    /// `approvals.jsonl` in the same directory.
    pub fn beside_storage(state_path: Option<&Path>) -> Self {
        Self::new(
            state_path
                .and_then(Path::parent)
                .map(|dir| dir.join("approvals.jsonl")),
        )
    }

    /// Records one decision. A failed write is reported, never absorbed:
    /// a supervision record the operator cannot see is worse than a loud
    /// complaint about one.
    pub fn record(
        &self,
        session: &SessionId,
        page: &PageId,
        request_id: &str,
        brief: &ApprovalBrief,
        outcome: &'static str,
        waited: std::time::Duration,
    ) {
        let Some(path) = &self.path else {
            return;
        };
        let entry = AuditEntry {
            at: now_rfc3339(),
            session: session.as_str().to_owned(),
            page: page.as_str().to_owned(),
            request_id: request_id.to_owned(),
            outcome,
            waited_ms: waited.as_millis(),
            class: brief.class.name().to_owned(),
            judged_url: brief.judged_url.clone(),
            basis: match &brief.basis {
                VerdictBasis::Rule { index, url_pattern } => match url_pattern {
                    Some(pattern) => format!("rule {index} ({pattern})"),
                    None => format!("rule {index}"),
                },
                VerdictBasis::SetDefault => "set_default".to_owned(),
                VerdictBasis::MissingUrl => "missing_url".to_owned(),
            },
            effect: effect_summary(brief),
        };
        let line = match serde_json::to_string(&entry) {
            Ok(line) => line,
            Err(error) => {
                eprintln!("rutter: cannot encode the approval audit record: {error}");
                return;
            }
        };

        if let Err(error) = append_line(path, &line) {
            eprintln!("rutter: cannot append to the approval audit {path:?}: {error}");
        }
    }
}

/// A one-line description of what a grant authorizes. The action's own
/// debug form is enough for an audit trail and keeps this module from
/// duplicating the serialization the dashboard renders.
fn effect_summary(brief: &ApprovalBrief) -> String {
    match &brief.effect {
        rutter_policy::ApprovalEffect::Action { action } => format!("{action:?}"),
        rutter_policy::ApprovalEffect::Cookies { count } => format!("{count} cookie write(s)"),
    }
}

/// Appends one line, creating the file and its directory on first use.
fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")
}

/// Current UTC time in RFC 3339.
fn now_rfc3339() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
