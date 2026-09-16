//! Input events dispatchable on a page through the engine.
//!
//! Boundary: the event vocabulary is protocol-neutral; translating these
//! into backend input commands is the engine implementation's job.

/// Mouse buttons an input event can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The primary (left) button.
    Left,
    /// The auxiliary (middle) button.
    Middle,
    /// The secondary (right) button.
    Right,
}

/// One input event dispatched to a page.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Moves the pointer to page coordinates.
    MouseMove {
        /// X coordinate in CSS pixels.
        x: f64,
        /// Y coordinate in CSS pixels.
        y: f64,
    },
    /// Presses a mouse button at page coordinates.
    MousePressed {
        /// X coordinate in CSS pixels.
        x: f64,
        /// Y coordinate in CSS pixels.
        y: f64,
        /// Button to press.
        button: MouseButton,
    },
    /// Releases a mouse button at page coordinates.
    MouseReleased {
        /// X coordinate in CSS pixels.
        x: f64,
        /// Y coordinate in CSS pixels.
        y: f64,
        /// Button to release.
        button: MouseButton,
    },
    /// Presses a key.
    KeyPressed {
        /// Key name in engine notation, for example `a`, `Enter`, `Tab`.
        key: String,
    },
    /// Releases a key.
    KeyReleased {
        /// Key name in engine notation, for example `a`, `Enter`, `Tab`.
        key: String,
    },
}
