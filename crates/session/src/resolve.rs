//! Reference resolution: snapshot handles back to live element boxes.
//!
//! Boundary: one evaluate round-trip per resolution. The page-side
//! script is owned by `rutter-observe` (`resolver_script`); this module
//! only evaluates it, parses the returned JSON defensively, and exposes
//! the box the auto-wait phases and pointing actions need.

use serde_json::Value;

use rutter_engine::error::EngineError;
use rutter_engine::page::PageHandle;
use rutter_observe::resolver_script;

/// The live geometry and state of a resolved element, viewport-relative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementBox {
    /// Left edge in CSS pixels.
    pub x: f64,
    /// Top edge in CSS pixels.
    pub y: f64,
    /// Box width in CSS pixels.
    pub width: f64,
    /// Box height in CSS pixels.
    pub height: f64,
    /// Whether the element reports disabled.
    pub disabled: bool,
    /// Whether the element computes to a hidden visibility.
    pub hidden: bool,
}

impl ElementBox {
    /// Center point of the box, the target of pointing actions.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Whether the box would render: non-empty and not hidden.
    pub fn visible(&self) -> bool {
        !self.hidden && self.width > 1.0 && self.height > 1.0
    }

    /// Stability key: rounded extents, equal when the element stopped
    /// moving between samples.
    fn stable_key(&self) -> (i64, i64, i64, i64) {
        (
            self.x.round() as i64,
            self.y.round() as i64,
            self.width.round() as i64,
            self.height.round() as i64,
        )
    }

    /// Whether two boxes count as stable samples of each other.
    pub fn stable_against(&self, other: &ElementBox) -> bool {
        self.stable_key() == other.stable_key()
    }
}

/// Resolves a reference through the page. `None` means the reference no
/// longer maps to a live element (`ReferenceExpired` for callers);
/// malformed page answers degrade to `None` instead of failing.
pub async fn resolve(
    page: &dyn PageHandle,
    reference: &str,
) -> Result<Option<ElementBox>, EngineError> {
    let raw = page.evaluate(&resolver_script(reference)).await?;
    Ok(parse_box(&raw))
}

/// Parses the resolver's answer; anything malformed counts as missing.
fn parse_box(raw: &Value) -> Option<ElementBox> {
    let object = raw.as_object()?;
    if object.get("missing").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if object.get("missing").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let field = |name: &str| object.get(name).and_then(Value::as_f64);
    Some(ElementBox {
        x: field("x")?,
        y: field("y")?,
        width: field("width")?,
        height: field("height")?,
        disabled: object
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hidden: object
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_well_formed_boxes() {
        let raw = json!({
            "missing": false,
            "x": 10.0, "y": 20.0, "width": 100.0, "height": 30.0,
            "disabled": false, "hidden": false
        });
        let element_box = parse_box(&raw).expect("box");
        assert_eq!(element_box.center(), (60.0, 35.0));
        assert!(element_box.visible());
        assert!(!element_box.disabled);
    }

    #[test]
    fn missing_and_malformed_both_degrade_to_none() {
        assert!(parse_box(&json!({ "missing": true })).is_none());
        assert!(parse_box(&json!("garbage")).is_none());
        assert!(parse_box(&json!({ "missing": false, "x": 1.0 })).is_none());
    }

    #[test]
    fn stability_requires_equal_rounded_extents() {
        let one = ElementBox {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 30.0,
            disabled: false,
            hidden: false,
        };
        let mut moved = one;
        moved.y = 21.0;
        let mut same = one;
        same.x = 10.2;
        assert!(one.stable_against(&same), "sub-pixel jitter is stable");
        assert!(!one.stable_against(&moved), "real movement is not stable");
    }
}
