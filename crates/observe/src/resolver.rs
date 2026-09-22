//! The in-page scripts owned by the observation layer: reference
//! resolution, focusing, option selection, text waits, and localStorage
//! dump/restore.
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

/// Builds a localStorage dump script: returns
/// `{ origin, data: {key: value, ...} }` for the current page. Hosts
/// where the storage API throws (opaque origins such as `file://`,
/// sandboxed frames) answer `{ unavailable: true }` instead of failing
/// the whole evaluate.
pub fn storage_dump_script() -> String {
    r#"(function () {
  'use strict';
  try {
    if (!window.localStorage) return { unavailable: true };
    var data = {};
    for (var i = 0; i < window.localStorage.length; i += 1) {
      var key = window.localStorage.key(i);
      data[key] = window.localStorage.getItem(key);
    }
    return { origin: window.location.origin, data: data };
  } catch (err) {
    return { unavailable: true };
  }
})();
"#
    .to_owned()
}

/// Builds a localStorage restore script from a JSON object of
/// key-value pairs; returns `{ restored: N }`.
pub fn storage_restore_script(entries_json: &str) -> String {
    RESTORE_TEMPLATE.replace("__ENTRIES__", entries_json)
}

/// Restore template; `__ENTRIES__` is a JSON object literal.
const RESTORE_TEMPLATE: &str = r#"(function () {
  'use strict';
  var ENTRIES = __ENTRIES__;
  var restored = 0;
  for (var key in ENTRIES) {
    try {
      window.localStorage.setItem(key, ENTRIES[key]);
      restored += 1;
    } catch (err) {}
  }
  return { restored: restored };
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
    fn storage_scripts_embed_their_payloads() {
        assert!(storage_dump_script().contains("localStorage"));
        let restore = storage_restore_script("{\"k\":\"v\"}");
        assert!(restore.contains("var ENTRIES = {\"k\":\"v\"}"));
    }

    #[test]
    fn scripts_are_ascii_only() {
        assert!(resolver_script("e17").is_ascii());
        assert!(focus_script("e17").is_ascii());
        assert!(select_script("e2", "[\"a\"]").is_ascii());
        assert!(wait_for_script("\"x\"").is_ascii());
    }
}
