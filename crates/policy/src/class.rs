//! Action classification for policy rules.

use rutter_core::action::Action;
use serde::{Deserialize, Serialize};

/// Coarse class of an operation, the axis policy rules match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    /// History and URL navigation.
    Navigation,
    /// Pointer actions on elements.
    Pointer,
    /// Keyboard text and key actions.
    Keyboard,
    /// Selecting options on form controls.
    Selection,
    /// Scrolling.
    Scroll,
    /// Cookie manipulation; sensitive by default (blueprint §7.6).
    Cookies,
}

impl ActionClass {
    /// Parses the class from its snake_case TOML name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "navigation" => Some(Self::Navigation),
            "pointer" => Some(Self::Pointer),
            "keyboard" => Some(Self::Keyboard),
            "selection" => Some(Self::Selection),
            "scroll" => Some(Self::Scroll),
            "cookies" => Some(Self::Cookies),
            _ => None,
        }
    }

    /// The snake_case TOML name of this class.
    pub fn name(self) -> &'static str {
        match self {
            Self::Navigation => "navigation",
            Self::Pointer => "pointer",
            Self::Keyboard => "keyboard",
            Self::Selection => "selection",
            Self::Scroll => "scroll",
            Self::Cookies => "cookies",
        }
    }
}

/// Maps an action onto its policy class.
pub fn class_of(action: &Action) -> ActionClass {
    match action {
        Action::Navigate { .. } | Action::Back | Action::Forward | Action::Reload => {
            ActionClass::Navigation
        }
        Action::Click { .. } | Action::Hover { .. } => ActionClass::Pointer,
        Action::Type { .. } | Action::PressKey { .. } => ActionClass::Keyboard,
        Action::SelectOption { .. } => ActionClass::Selection,
        Action::Scroll { .. } => ActionClass::Scroll,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rutter_core::reference::Reference;

    #[test]
    fn classes_round_trip_through_names() {
        for class in [
            ActionClass::Navigation,
            ActionClass::Pointer,
            ActionClass::Keyboard,
            ActionClass::Selection,
            ActionClass::Scroll,
            ActionClass::Cookies,
        ] {
            assert_eq!(ActionClass::parse(class.name()), Some(class));
        }
        assert_eq!(ActionClass::parse("nonsense"), None);
    }

    #[test]
    fn actions_map_to_expected_classes() {
        let reference = Reference::new("e1");
        assert_eq!(class_of(&Action::Back), ActionClass::Navigation);
        assert_eq!(
            class_of(&Action::Click {
                reference: reference.clone()
            }),
            ActionClass::Pointer
        );
        assert_eq!(
            class_of(&Action::Type {
                reference: reference.clone(),
                text: "x".to_owned()
            }),
            ActionClass::Keyboard
        );
        assert_eq!(
            class_of(&Action::SelectOption {
                reference,
                values: vec!["a".to_owned()]
            }),
            ActionClass::Selection
        );
    }
}
