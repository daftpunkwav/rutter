//! Opaque identifiers for the domain objects defined in the glossary.
//!
//! Boundary: identifiers are opaque strings with no internal structure;
//! minting new identifiers is the responsibility of the layer that owns
//! the object's lifecycle (for example `rutter-session`). Callers must
//! not parse or construct meaning from the string content.
//!
//! Opacity is about *reading* an identifier, not about minting freedom:
//! the layer that owns the lifecycle may impose constraints a conforming
//! identifier must satisfy. A `SessionId` names the session's on-disk
//! storage file, so the session layer refuses ids that cannot compose
//! into one — empty, longer than 200 bytes, or carrying a path
//! separator or NUL (see `rutter-session`'s manager). The shipped
//! transports mint `stdio-<pid>` and `http-<pid>-<serial>`, which
//! always pass.

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

/// Id length beyond which the session's suffixed storage file name
/// cannot fit a directory entry (the common 255-byte limit, rounded
/// down).
const MAX_ID_BYTES: usize = 200;

impl SessionId {
    /// Whether the identifier can safely name the session's on-disk
    /// storage file: non-empty, at most 200 bytes long, and free of
    /// path separators and NUL. The session layer refuses ids that
    /// fail this check before the engine ever starts — the id becomes
    /// part of a file name under the state directory, and a separator
    /// inside it would steer that file out of the directory.
    pub fn is_storage_safe(&self) -> bool {
        let name = self.as_str();
        !(name.is_empty() || name.len() > MAX_ID_BYTES || name.contains(['/', '\\', '\0']))
    }
}

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

    #[test]
    fn storage_safety_refuses_separators_emptiness_and_overlong_ids() {
        // The constraint the session layer enforces before the engine
        // ever starts; see `rutter-session`'s manager for the refusal.
        for id in ["", "../evil", "a/b", "a\\b", "a\0b"] {
            assert!(
                !SessionId::new(id.to_owned()).is_storage_safe(),
                "session id {id:?} must not name a storage file"
            );
        }
        assert!(!SessionId::new("x".repeat(MAX_ID_BYTES + 1)).is_storage_safe());
        assert!(SessionId::new("x".repeat(MAX_ID_BYTES)).is_storage_safe());
        assert!(SessionId::new("stdio-4711").is_storage_safe());
    }
}
