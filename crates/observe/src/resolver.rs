//! The in-page element resolver used by the action executor.
//!
//! Boundary: page-side JS owned by the observation layer, next to the
//! serializer. The resolver maps a snapshot reference back to its live
//! element, scrolls it into view, and reports its box plus enabled and
//! visibility state so the executor can run its auto-wait phases. All
//! page JS stays ASCII-only and never throws.

/// Builds the resolver script for one reference.
///
/// The reference is sanitized to its alphanumeric core before being
/// embedded, so a hostile reference can only resolve to nothing, never
/// execute. Evaluate it in the page and read the returned object:
/// `{ missing: true }` when the reference is gone, otherwise the box
/// (`x`, `y`, `width`, `height` in viewport-relative CSS pixels),
/// `disabled`, and `hidden`.
pub fn resolver_script(reference: &str) -> String {
    let sanitized: String = reference
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    RESOLVER_TEMPLATE.replace("__REF__", &sanitized)
}

/// The resolver template; `__REF__` is replaced with a sanitized ref.
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
    fn script_is_ascii_only() {
        assert!(resolver_script("e17").is_ascii());
    }
}
