//! Embedded JS assets owned by the observation layer.
//!
//! Boundary: static asset text only. The assets are the serializer, the
//! reader, and the document-start entropy capture; all must stay
//! ASCII-only and carry a truthful header (the CI encoding and header
//! gates apply to them like any other source file).

/// The in-page DOM serializer (see `assets/serializer.js`).
pub const SERIALIZER_JS: &str = include_str!("assets/serializer.js");

/// The in-page markdown reader (see `assets/reader.js`).
pub const READER_JS: &str = include_str!("assets/reader.js");

/// The document-start entropy capture (see `assets/entropy.js`).
pub const ENTROPY_JS: &str = include_str!("assets/entropy.js");

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
    fn the_page_scripts_have_a_behavioural_suite() {
        // These constants used to be pinned by string assertions --
        // `SERIALIZER_JS.contains("function pruneRefs")` and
        // `!READER_JS.contains("cells.slice(0")`. Neither could tell
        // a working rule from a dead one: a rule that was renamed, or
        // one that was defined and never called, passed both. The
        // scripts are now executed instead (see `tests/js/` and
        // `crates/observe/tests/page_scripts.rs`); what is left here
        // is that the suites they run still name these assets, so a
        // moved file fails the gate instead of silently emptying it.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tests")
            .join("js");
        for suite in ["serializer.test.mjs", "reader.test.mjs"] {
            let path = root.join(suite);
            assert!(path.is_file(), "the suite {} is missing", path.display());
            let source = std::fs::read_to_string(&path).expect("the suite is readable");
            assert!(
                source.contains("assets/serializer.js") || source.contains("assets/reader.js"),
                "{suite} must load a shipped asset, not a copy"
            );
        }
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

    #[test]
    fn entropy_capture_names_the_property_the_serializer_reads() {
        // The two assets meet at one name: the capture locks it onto the
        // global, the serializer calls it. A rename on either side would
        // silently drop every document back to a page-replaceable
        // source, so it is asserted rather than assumed.
        assert!(ENTROPY_JS.contains("__rutterRefScope"));
        assert!(SERIALIZER_JS.contains("__rutterRefScope"));
        assert!(ENTROPY_JS.is_ascii(), "the capture must stay ASCII-only");
        assert!(
            ENTROPY_JS.contains("configurable: false") && ENTROPY_JS.contains("writable: false"),
            "the capture must lock the property it defines"
        );
        assert!(
            !ENTROPY_JS.contains(".toString(") && !ENTROPY_JS.contains("for (const"),
            "the conversion must not go through a prototype a page can replace"
        );
    }
}
