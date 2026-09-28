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
    fn the_reverse_ref_store_sweeps_collected_entries() {
        // The reverse store is a strong Map keyed by the ref string, so
        // nothing in it can be collected while its entry lives: without
        // a sweep it grows with every element the page has ever shown,
        // and a long-lived document that re-renders grows the browser
        // process with it.
        //
        // Behaviour is covered by the page-side harness; these
        // assertions pin that the sweep exists, runs on the way out of
        // the store accessor, and actually deletes what it finds.
        assert!(
            SERIALIZER_JS.contains("function pruneRefs"),
            "the serializer must sweep collected refs"
        );
        assert!(
            SERIALIZER_JS.contains("pruneRefs(window.__rutterRefStore)"),
            "every snapshot that reaches the accessor sweeps first"
        );
        assert!(
            SERIALIZER_JS.contains("store.reverse.delete(collected[i])"),
            "a sweep that finds nothing to remove is not a sweep"
        );
    }

    #[test]
    fn the_reader_never_clips_a_row_to_the_header() {
        // A colspan header spans fewer cells than the rows under it, and
        // clipping those rows to the header's cell count dropped their
        // trailing cells while the readout still claimed to be complete.
        assert!(
            !READER_JS.contains("cells.slice(0"),
            "data cells must not be clipped to the header width"
        );
        assert!(
            READER_JS.contains("while (cells.length < width)"),
            "rows are padded to the widest row instead"
        );
        assert!(
            READER_JS.contains("if (headerIndex === -1) return;"),
            "a leading row with no cells must not swallow the table"
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
