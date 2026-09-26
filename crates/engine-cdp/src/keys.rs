//! Key-name mapping for keyboard events.
//!
//! Boundary: translates the engine's key notation (`a`, `Enter`, `Tab`,
//! `ArrowLeft`, …) onto the full CDP key descriptor — `key`, `code`,
//! `windowsVirtualKeyCode`, and the `text` that makes printable keys
//! actually land in inputs. The table covers the keys a US-layout agent
//! keyboard produces; an unknown key keeps the name-only event of older
//! versions rather than failing.

use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};

/// What a key press carries: the physical-key `code`, the Windows
/// virtual key code the web platform expects, and the text the key
/// inserts when pressed (empty for keys that insert nothing).
pub(crate) struct KeyDescriptor {
    code: String,
    windows_virtual_key_code: i64,
    text: Option<String>,
}

/// Looks up the descriptor for one key in the engine's notation; `None`
/// keeps the name-only event.
pub(crate) fn descriptor(key: &str) -> Option<KeyDescriptor> {
    // Single printable ASCII characters: letters and digits follow
    // their uniform schemes, punctuation keeps its US-layout codes.
    let bytes = key.as_bytes();
    if bytes.len() == 1 {
        let character = bytes[0];
        return Some(match character {
            b'a'..=b'z' | b'A'..=b'Z' => KeyDescriptor {
                code: format!("Key{}", character.to_ascii_uppercase() as char),
                windows_virtual_key_code: i64::from(character.to_ascii_uppercase()),
                text: Some((character as char).to_string()),
            },
            b'0'..=b'9' => KeyDescriptor {
                code: format!("Digit{}", character as char),
                windows_virtual_key_code: i64::from(character),
                text: Some((character as char).to_string()),
            },
            b' ' => KeyDescriptor {
                code: "Space".to_owned(),
                windows_virtual_key_code: 32,
                text: Some(" ".to_owned()),
            },
            other => punctuation(other)?,
        });
    }
    named(key)
}

/// US-layout punctuation codes; `None` for characters the layout does
/// not name this way.
fn punctuation(character: u8) -> Option<KeyDescriptor> {
    let (code, virtual_key, text) = match character as char {
        ';' => ("Semicolon", 186, ";"),
        '=' => ("Equal", 187, "="),
        ',' => ("Comma", 188, ","),
        '-' => ("Minus", 189, "-"),
        '.' => ("Period", 190, "."),
        '/' => ("Slash", 191, "/"),
        '`' => ("Backquote", 192, "`"),
        '[' => ("BracketLeft", 219, "["),
        '\\' => ("Backslash", 220, "\\"),
        ']' => ("BracketRight", 221, "]"),
        '\'' => ("Quote", 222, "'"),
        _ => return None,
    };
    Some(KeyDescriptor {
        code: code.to_owned(),
        windows_virtual_key_code: virtual_key,
        text: Some(text.to_owned()),
    })
}

/// Named keys in the engine notation; the names match the `code` and
/// `key` values of the web platform.
fn named(key: &str) -> Option<KeyDescriptor> {
    let (code, virtual_key, text) = match key {
        "Enter" => ("Enter", 13, "\r"),
        "Backspace" => ("Backspace", 8, ""),
        "Tab" => ("Tab", 9, "\t"),
        "Escape" => ("Escape", 27, ""),
        "Space" => ("Space", 32, " "),
        "PageUp" => ("PageUp", 33, ""),
        "PageDown" => ("PageDown", 34, ""),
        "End" => ("End", 35, ""),
        "Home" => ("Home", 36, ""),
        "ArrowLeft" => ("ArrowLeft", 37, ""),
        "ArrowUp" => ("ArrowUp", 38, ""),
        "ArrowRight" => ("ArrowRight", 39, ""),
        "ArrowDown" => ("ArrowDown", 40, ""),
        "Insert" => ("Insert", 45, ""),
        "Delete" => ("Delete", 46, ""),
        "F1" => ("F1", 112, ""),
        "F2" => ("F2", 113, ""),
        "F3" => ("F3", 114, ""),
        "F4" => ("F4", 115, ""),
        "F5" => ("F5", 116, ""),
        "F6" => ("F6", 117, ""),
        "F7" => ("F7", 118, ""),
        "F8" => ("F8", 119, ""),
        "F9" => ("F9", 120, ""),
        "F10" => ("F10", 121, ""),
        "F11" => ("F11", 122, ""),
        "F12" => ("F12", 123, ""),
        _ => return None,
    };
    Some(KeyDescriptor {
        code: code.to_owned(),
        windows_virtual_key_code: virtual_key,
        text: if text.is_empty() {
            None
        } else {
            Some(text.to_owned())
        },
    })
}

/// Builds a CDP key event for a named key: known keys carry `code`,
/// the virtual key code, and (where the key inserts text) `text`;
/// unknown keys degrade to the name-only event.
pub fn key_params(event_type: DispatchKeyEventType, key: &str) -> DispatchKeyEventParams {
    let inserts = matches!(event_type, DispatchKeyEventType::KeyDown);
    let mut params = DispatchKeyEventParams::new(event_type);
    params.key = Some(key.to_owned());
    if let Some(descriptor) = descriptor(key) {
        params.code = Some(descriptor.code);
        params.windows_virtual_key_code = Some(descriptor.windows_virtual_key_code);
        params.native_virtual_key_code = Some(descriptor.windows_virtual_key_code);
        // Text rides on the down-phase only: the up-phase with text
        // would insert the character a second time.
        if inserts {
            params.text = descriptor.text;
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_carry_code_virtual_key_and_text() {
        let params = key_params(DispatchKeyEventType::KeyDown, "a");
        assert_eq!(params.code.as_deref(), Some("KeyA"));
        assert_eq!(params.windows_virtual_key_code, Some(65));
        assert_eq!(params.text.as_deref(), Some("a"));

        let up = key_params(DispatchKeyEventType::KeyUp, "a");
        assert_eq!(up.text, None, "text rides on the down-phase only");
    }

    #[test]
    fn digits_and_punctuation_map_to_the_us_layout() {
        let five = key_params(DispatchKeyEventType::KeyDown, "5");
        assert_eq!(five.code.as_deref(), Some("Digit5"));
        assert_eq!(five.windows_virtual_key_code, Some(53));

        let comma = key_params(DispatchKeyEventType::KeyDown, ",");
        assert_eq!(comma.code.as_deref(), Some("Comma"));
        assert_eq!(comma.text.as_deref(), Some(","));
    }

    #[test]
    fn enter_inserts_a_carriage_return() {
        let params = key_params(DispatchKeyEventType::KeyDown, "Enter");
        assert_eq!(params.windows_virtual_key_code, Some(13));
        assert_eq!(params.code.as_deref(), Some("Enter"));
        assert_eq!(params.text.as_deref(), Some("\r"));
    }

    #[test]
    fn arrows_and_function_keys_carry_codes_without_text() {
        let left = key_params(DispatchKeyEventType::KeyDown, "ArrowLeft");
        assert_eq!(left.windows_virtual_key_code, Some(37));
        assert_eq!(left.text, None);

        let f5 = key_params(DispatchKeyEventType::KeyDown, "F5");
        assert_eq!(f5.windows_virtual_key_code, Some(116));
    }

    #[test]
    fn unknown_keys_degrade_to_name_only() {
        let params = key_params(DispatchKeyEventType::KeyDown, "MediaPlay");
        assert_eq!(params.key.as_deref(), Some("MediaPlay"));
        assert_eq!(params.code, None);
        assert_eq!(params.windows_virtual_key_code, None);
    }
}
