//! Opaque identifiers for the domain objects defined in the glossary.
//!
//! Boundary: identifiers are opaque strings with no internal structure;
//! minting new identifiers is the responsibility of the layer that owns
//! the object's lifecycle (for example `rutter-session`). Callers must
//! not parse or construct meaning from the string content.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! opaque_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            /// Creates an identifier from an already-minted value.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the identifier's opaque string content.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id!(
    /// Identifies one MCP client's workspace: its context, pages, and state.
    SessionId
);
opaque_id!(
    /// Identifies one isolated cookie/storage unit inside an engine.
    ContextId
);
opaque_id!(
    /// Identifies one tab/target inside a context.
    PageId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_displays_its_value() {
        let id = SessionId::new("session-1");
        assert_eq!(id.as_str(), "session-1");
        assert_eq!(id.to_string(), "session-1");
    }

    #[test]
    fn identifiers_round_trip_through_strings() {
        // Each identifier is its own type on purpose: mixing a page with a
        // context identifier must be a compile error, not a runtime one.
        let page = PageId::new("1");
        let context = ContextId::new("2");
        let session = SessionId::new("3");
        assert_eq!(page.as_str(), "1");
        assert_eq!(context.as_str(), "2");
        assert_eq!(session.as_str(), "3");
    }
}
