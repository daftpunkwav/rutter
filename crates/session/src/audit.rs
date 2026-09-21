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

#[cfg(test)]
mod tests {
    //! The audit's write paths: every verdict basis as it lands on disk,
    //! the no-op audit a session without a state directory gets, and the
    //! loud failure a blocked append reports instead of absorbing.

    use super::*;
    use rutter_policy::{ActionClass, ApprovalEffect};

    fn brief(basis: VerdictBasis) -> ApprovalBrief {
        ApprovalBrief {
            class: ActionClass::Cookies,
            judged_url: Some("https://shop.example/".to_owned()),
            basis,
            effect: ApprovalEffect::Cookies { count: 2 },
        }
    }

    fn read_single_line(path: &Path) -> serde_json::Value {
        let text = std::fs::read_to_string(path).expect("the audit file exists");
        let line = text.lines().next().expect("one line");
        serde_json::from_str(line).expect("valid JSON")
    }

    #[test]
    fn bases_land_on_disk_in_human_quotable_form() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("approvals.jsonl");
        let audit = ApprovalAudit::new(Some(path.clone()));

        let cases: Vec<(VerdictBasis, &str)> = vec![
            (
                VerdictBasis::Rule {
                    index: 3,
                    url_pattern: Some("https://shop.example/*".to_owned()),
                },
                "rule 3 (https://shop.example/*)",
            ),
            (
                VerdictBasis::Rule {
                    index: 2,
                    url_pattern: None,
                },
                "rule 2",
            ),
            (VerdictBasis::SetDefault, "set_default"),
            (VerdictBasis::MissingUrl, "missing_url"),
        ];

        for (basis, _) in &cases {
            audit.record(
                &SessionId::new("s1"),
                &PageId::new("p1"),
                "apr-9",
                &brief(basis.clone()),
                DENIED,
                std::time::Duration::from_millis(12),
            );
        }

        let text = std::fs::read_to_string(path).expect("the audit file exists");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "one decision, one line");
        for (line, (_, expected)) in lines.iter().zip(&cases) {
            let record: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
            assert_eq!(record["basis"], *expected);
            assert_eq!(record["outcome"], "denied");
            assert_eq!(record["waited_ms"], 12);
            assert_eq!(record["judged_url"], "https://shop.example/");
            assert_eq!(record["effect"], "2 cookie write(s)");
        }
    }

    #[test]
    fn a_rule_basis_with_an_action_effect_uses_the_debug_form() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("approvals.jsonl");
        let audit = ApprovalAudit::new(Some(path.clone()));
        let brief = ApprovalBrief {
            class: ActionClass::Navigation,
            judged_url: None,
            basis: VerdictBasis::SetDefault,
            effect: ApprovalEffect::Action {
                action: rutter_core::action::Action::Navigate {
                    url: "https://shop.example/".to_owned(),
                },
            },
        };
        audit.record(
            &SessionId::new("s1"),
            &PageId::new("p1"),
            "apr-1",
            &brief,
            TIMED_OUT,
            std::time::Duration::ZERO,
        );
        let record = read_single_line(&path);
        assert!(
            record["effect"]
                .as_str()
                .is_some_and(|effect| effect.contains("Navigate")),
            "the action's debug form is the audit summary: {record}"
        );
        assert_eq!(record["judged_url"], serde_json::Value::Null);
    }

    #[test]
    fn a_missing_state_directory_disables_the_trail() {
        let audit = ApprovalAudit::new(None);
        // Must simply not write anywhere; the call contract is infallible.
        audit.record(
            &SessionId::new("s1"),
            &PageId::new("p1"),
            "apr-1",
            &brief(VerdictBasis::SetDefault),
            CANCELLED,
            std::time::Duration::ZERO,
        );
    }

    #[test]
    fn the_first_record_creates_the_directory_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/deeper/approvals.jsonl");
        let audit = ApprovalAudit::new(Some(path.clone()));
        audit.record(
            &SessionId::new("s1"),
            &PageId::new("p1"),
            "apr-1",
            &brief(VerdictBasis::SetDefault),
            GRANTED,
            std::time::Duration::ZERO,
        );
        assert!(path.exists(), "append creates the tree on first use");
    }

    #[test]
    fn a_blocked_append_is_reported_not_absorbed() {
        // The parent exists but is a file: create_dir_all fails. The
        // record call must not panic; the error goes to stderr (asserted
        // by inspection, the contract here is "never silently skip").
        let dir = tempfile::tempdir().expect("tempdir");
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, "not a directory").expect("blocker file");
        let audit = ApprovalAudit::new(Some(blocker.join("approvals.jsonl")));
        audit.record(
            &SessionId::new("s1"),
            &PageId::new("p1"),
            "apr-1",
            &brief(VerdictBasis::SetDefault),
            GRANTED,
            std::time::Duration::ZERO,
        );
    }
}
