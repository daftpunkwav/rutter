//! Embedded JS assets owned by the observation layer.
//!
//! Boundary: static asset text only. The serializer script is the sole
//! asset; it must stay ASCII-only and carry a truthful header (the CI
//! encoding and header gates apply to it like any other source file).

/// The in-page DOM serializer (see `assets/serializer.js`).
pub const SERIALIZER_JS: &str = include_str!("assets/serializer.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializer_script_is_embedded_non_empty() {
        assert!(SERIALIZER_JS.len() > 1000, "script looks truncated");
        assert!(SERIALIZER_JS.contains("__rutterRefStore"));
    }

    #[test]
    fn serializer_script_is_ascii_only() {
        assert!(
            SERIALIZER_JS.is_ascii(),
            "the serializer must stay ASCII-only for the encoding gate"
        );
    }
}
