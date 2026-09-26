//! Embedded JS assets owned by the observation layer.
//!
//! Boundary: static asset text only. The assets are the serializer and
//! the reader scripts; both must stay ASCII-only and carry a truthful
//! header (the CI encoding and header gates apply to them like any
//! other source file).

/// The in-page DOM serializer (see `assets/serializer.js`).
pub const SERIALIZER_JS: &str = include_str!("assets/serializer.js");

/// The in-page markdown reader (see `assets/reader.js`).
pub const READER_JS: &str = include_str!("assets/reader.js");

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

    #[test]
    fn reader_script_is_embedded_non_empty() {
        assert!(READER_JS.len() > 1000, "script looks truncated");
        assert!(READER_JS.contains("markdown"));
    }

    #[test]
    fn reader_script_is_ascii_only() {
        assert!(
            READER_JS.is_ascii(),
            "the reader must stay ASCII-only for the encoding gate"
        );
    }
}
