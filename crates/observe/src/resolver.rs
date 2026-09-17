//! The in-page scripts owned by the observation layer: reference
//! resolution, focusing, option selection, and text waits.
//!
//! Boundary: page-side JS next to the serializer. These scripts map
//! snapshot references back to live elements and drive small page
//! mutations the action executor needs. All page JS stays ASCII-only,
//! sanitizes hostile references, and never throws.

/// Builds the resolver script for one reference.
///
/// The reference is sanitized to its alphanumeric core before being
/// embedded, so a hostile reference can only resolve to nothing, never
/// execute. Evaluate it in the page and read the returned object:
/// `{ missing: true }` when the reference is gone, otherwise the box
/// (`x`, `y`, `width`, `height` in viewport-relative CSS pixels),
/// `disabled`, and `hidden`.
pub fn resolver_script(reference: &str) -> String {
    resolve_template(reference, "false")
}

/// Like [`resolver_script`], but focuses the element before measuring;
/// the `type` action focuses so inserted text lands in the field.
pub fn focus_script(reference: &str) -> String {
    resolve_template(reference, "true")
}

/// Builds a select script: selects options whose `value` is in
/// `values_json` (a JSON array literal) and dispatches `input` and
/// `change`. Returns `{ missing }`, `{ not_select }`, or `{ matched }`.
pub fn select_script(reference: &str, values_json: &str) -> String {
    let sanitized: String = reference
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    SELECT_TEMPLATE
        .replace("__REF__", &sanitized)
        .replace("__VALUES__", values_json)
}

/// Builds a text-wait script; `text_json` is the needle as a JSON
/// string literal. Returns `{ found }`.
pub fn wait_for_script(text_json: &str) -> String {
    WAIT_TEMPLATE.replace("__TEXT__", text_json)
}

fn resolve_template(reference: &str, focus: &str) -> String {
    let sanitized: String = reference
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    RESOLVER_TEMPLATE
        .replace("__REF__", &sanitized)
        .replace("__FOCUS__", focus)
}

/// The resolver template; `__REF__` is replaced with a sanitized ref and
/// `__FOCUS__` with a boolean literal.
const RESOLVER_TEMPLATE: &str = r#"(function () {
  'use strict';
  var REF = "__REF__";
  var store = window.__rutterRefStore;
  if (!store || !store.reverse) return { missing: true };
  var weak = store.reverse.get(REF);
  var el = weak && weak.deref ? weak.deref() : null;
  if (!el || !el.isConnected) return { missing: true };
  try {
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  } catch (err) {}
  try {
    if (__FOCUS__) el.focus();
  } catch (err) {}
  var box = el.getBoundingClientRect();
  var style = null;
  try {
    style = window.getComputedStyle(el);
  } catch (err) {}
  var disabled = false;
  try {
    disabled = Boolean(el.disabled) || el.getAttribute('aria-disabled') === 'true';
  } catch (err) {}
  return {
    missing: false,
    x: box.left,
    y: box.top,
    width: box.width,
    height: box.height,
    disabled: disabled,
    hidden: !style || style.visibility === 'hidden' || style.visibility === 'collapse'
  };
})();
"#;

/// Select template: reports `missing`, `not_select`, or the match count.
const SELECT_TEMPLATE: &str = r#"(function () {
  'use strict';
  var REF = "__REF__";
  var VALUES = __VALUES__;
  var store = window.__rutterRefStore;
  if (!store || !store.reverse) return { missing: true };
  var weak = store.reverse.get(REF);
  var el = weak && weak.deref ? weak.deref() : null;
  if (!el || !el.isConnected) return { missing: true };
  if (el.tagName !== 'SELECT') return { not_select: true };
  var matched = 0;
  Array.prototype.forEach.call(el.options, function (option) {
    option.selected = VALUES.indexOf(option.value) !== -1;
    if (option.selected) matched += 1;
  });
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { missing: false, not_select: false, matched: matched };
})();
"#;

/// Text-wait template: reports whether the body text contains the needle.
const WAIT_TEMPLATE: &str = r#"(function () {
  'use strict';
  var NEEDLE = __TEXT__;
  var body = document.body;
  return { found: Boolean(body) && body.innerText.indexOf(NEEDLE) !== -1 };
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_sanitized_reference() {
        let script = resolver_script("e17");
        assert!(script.contains("\"e17\""));
    }

    #[test]
    fn hostile_references_cannot_break_out() {
        let script = resolver_script("e1\"); process.exit();//");
        assert!(
            !script.contains("process.exit();\""),
            "injection payload gone"
        );
        assert!(script.contains("\"e1processexit\""), "alphanumerics kept");
    }

    #[test]
    fn focus_script_flips_the_focus_flag() {
        assert!(focus_script("e1").contains("if (true) el.focus();"));
        assert!(resolver_script("e1").contains("if (false) el.focus();"));
    }

    #[test]
    fn select_script_embeds_values_as_json() {
        let script = select_script("e2", &serde_json::to_string(&["a", "b"]).expect("json"));
        assert!(script.contains("var VALUES = [\"a\",\"b\"]"));
    }

    #[test]
    fn wait_script_embeds_the_needle_literal() {
        let script = wait_for_script(&serde_json::to_string("done").expect("json"));
        assert!(script.contains("var NEEDLE = \"done\""));
    }

    #[test]
    fn scripts_are_ascii_only() {
        assert!(resolver_script("e17").is_ascii());
        assert!(focus_script("e17").is_ascii());
        assert!(select_script("e2", "[\"a\"]").is_ascii());
        assert!(wait_for_script("\"x\"").is_ascii());
    }
}
