//! Reader envelope parsing: page JSON to readout conversion entry.
//!
//! Boundary: adapts the in-page reader's envelope to
//! [`rutter_core::readout::Readout`]. The caller evaluates
//! `reader_script()` in a page and hands the returned JSON here; the
//! engine stays out of this crate entirely.

use serde_json::Value;

use rutter_core::readout::Readout;

/// Reader envelope format version accepted by this build.
const ENVELOPE_VERSION: f64 = 1.0;

/// Hard clamp for the title. The script caps at the same value; this
/// defends against a misbehaving or hostile page.
const TITLE_LIMIT: usize = 200;

/// Hard clamp for the markdown body, mirroring the script's own size
/// guard. Clamping sets `truncated` rather than failing.
const MARKDOWN_LIMIT: usize = 100_000;

/// Converts a reader response into a readout.
///
/// The response may be the envelope object itself or a JSON string
/// containing it. Missing or malformed envelope parts degrade: an
/// unreadable response yields an empty, truncated readout rather than
/// an error; overlong strings are clamped and set `truncated`.
pub fn read_from_response(response: &Value) -> Readout {
    // Borrow the envelope when possible; only a JSON-string envelope
    // needs an owned parse (mirrors `snapshot_from_response`).
    let parsed;
    let envelope = match response {
        Value::Object(_) => response,
        Value::String(text) => {
            parsed = serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
            &parsed
        }
        _ => {
            parsed = Value::Null;
            &parsed
        }
    };

    let object = envelope.as_object();
    let version_ok = object
        .and_then(|object| object.get("version"))
        .and_then(Value::as_f64)
        .is_none_or(|version| version <= ENVELOPE_VERSION);
    let reader_flagged = object
        .and_then(|object| object.get("truncated"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let present = object.is_some();

    let mut truncated = !version_ok || !present;
    let title = clamp(
        object
            .and_then(|object| object.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        TITLE_LIMIT,
        &mut truncated,
    );
    let markdown = clamp(
        object
            .and_then(|object| object.get("markdown"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        MARKDOWN_LIMIT,
        &mut truncated,
    );

    Readout {
        title,
        markdown,
        truncated: truncated || reader_flagged,
    }
}

/// Clamps `text` to `limit` characters, setting `truncated` when the
/// clamp bites. Character-counted, so no UTF-8 boundary can split.
fn clamp(text: &str, limit: usize, truncated: &mut bool) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    *truncated = true;
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_envelope_object() {
        let response = json!({
            "version": 1,
            "truncated": false,
            "title": "Example",
            "markdown": "# Example\n\nHello."
        });
        let readout = read_from_response(&response);
        assert_eq!(readout.title, "Example");
        assert_eq!(readout.markdown, "# Example\n\nHello.");
        assert!(!readout.truncated);
    }

    #[test]
    fn converts_json_string_envelope() {
        let response = Value::String(r#"{"version":1,"title":"t","markdown":"m"}"#.to_owned());
        let readout = read_from_response(&response);
        assert_eq!(readout.title, "t");
        assert_eq!(readout.markdown, "m");
        assert!(!readout.truncated);
    }

    #[test]
    fn unreadable_response_degrades_to_empty_truncated() {
        for response in [Value::Null, json!(42), json!("not json")] {
            let readout = read_from_response(&response);
            assert_eq!(readout.title, "");
            assert_eq!(readout.markdown, "");
            assert!(readout.truncated);
        }
    }

    #[test]
    fn newer_version_flags_truncated() {
        let response = json!({"version": 2, "title": "t", "markdown": "m"});
        let readout = read_from_response(&response);
        assert_eq!(readout.markdown, "m");
        assert!(readout.truncated);
    }

    #[test]
    fn reader_truncation_flag_is_carried() {
        let response = json!({"version": 1, "truncated": true, "title": "", "markdown": "part"});
        assert!(read_from_response(&response).truncated);
    }

    #[test]
    fn overlong_fields_are_clamped_and_flag_truncated() {
        let response = json!({
            "version": 1,
            "title": "a".repeat(TITLE_LIMIT + 1),
            "markdown": "b".repeat(MARKDOWN_LIMIT + 1)
        });
        let readout = read_from_response(&response);
        assert_eq!(readout.title.chars().count(), TITLE_LIMIT);
        assert_eq!(readout.markdown.chars().count(), MARKDOWN_LIMIT);
        assert!(readout.truncated);
    }

    #[test]
    fn multibyte_clamp_stays_on_char_boundaries() {
        let response = json!({
            "version": 1,
            "title": "",
            "markdown": "\u{1F600}".repeat(MARKDOWN_LIMIT + 10)
        });
        let readout = read_from_response(&response);
        assert_eq!(readout.markdown.chars().count(), MARKDOWN_LIMIT);
        assert!(readout.truncated);
    }
}
