//! Markdown renderings of pages for agents to read.
//!
//! Boundary: pure data plus its text rendering. Extracting the
//! markdown from a page lives in `rutter-observe` (in-page reader
//! script plus envelope conversion); the extraction rules are
//! specified by `docs/read-format.md`. The rendering here is one
//! metadata line for the page title, then the markdown body.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A page's readable content extracted as a markdown document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Readout {
    /// Title of the page (`document.title`), whitespace-collapsed.
    pub title: String,
    /// The markdown body. Links, images, and tables carry absolute
    /// URLs; hostile page data is clamped, never fatal.
    pub markdown: String,
    /// Whether the extraction hit a guard (node, depth, or size cap)
    /// and therefore omits part of the page.
    pub truncated: bool,
}

impl fmt::Display for Readout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.title)?;
        writeln!(f)?;
        write!(f, "{}", self.markdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_renders_title_then_body() {
        let readout = Readout {
            title: "Example".to_owned(),
            markdown: "# Example".to_owned(),
            truncated: false,
        };
        assert_eq!(readout.to_string(), "Example\n\n# Example");
    }

    #[test]
    fn readout_travels_as_typed_vocabulary() {
        // Serialization conventions (docs/glossary.md): UTF-8 JSON for
        // anything embedding in event payloads or logs.
        let readout = Readout {
            title: "t".to_owned(),
            markdown: "m".to_owned(),
            truncated: true,
        };
        let json = serde_json::to_value(&readout).expect("serializable");
        assert_eq!(
            json,
            serde_json::json!({
                "title": "t",
                "markdown": "m",
                "truncated": true
            })
        );
    }
}
