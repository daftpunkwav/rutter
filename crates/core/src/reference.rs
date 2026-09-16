//! Stable handles to elements inside snapshots.
//!
//! Boundary: references are minted by the observation layer from engine
//! backend node ids and stay usable across snapshots of the same page
//! where possible. Callers treat them as opaque tokens passed back inside
//! actions; resolution to a concrete node happens below, in the engine
//! implementation.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A stable handle to an element, usable across snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reference(String);

impl Reference {
    /// Creates a reference from an already-minted backend value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the reference's opaque string content.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_displays_its_value() {
        let reference = Reference::new("e17");
        assert_eq!(reference.as_str(), "e17");
        assert_eq!(reference.to_string(), "e17");
    }
}
