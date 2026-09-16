//! CDP error folding: chromiumoxide failures become `EngineError`.
//!
//! Boundary: one place maps third-party errors onto the engine error
//! contract so the rest of the crate stays mapping-free and no CDP
//! error type escapes into a public signature.

use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    MouseButton as CdpMouseButton,
};
use chromiumoxide::error::CdpError;

use rutter_engine::error::EngineError;
use rutter_engine::input::MouseButton;

/// Folds a chromiumoxide error into the engine error contract.
pub fn fold(error: CdpError) -> EngineError {
    match error {
        CdpError::Timeout => EngineError::Timeout {
            operation: "cdp command".to_owned(),
            elapsed: Duration::ZERO,
        },
        CdpError::NoResponse => EngineError::Terminated,
        CdpError::LaunchExit(_, _) | CdpError::LaunchTimeout(_) | CdpError::LaunchIo(_, _) => {
            EngineError::LaunchFailed {
                detail: error.to_string(),
            }
        }
        CdpError::Chrome(inner) => EngineError::Internal {
            detail: format!("cdp command failed: {inner}"),
        },
        other => EngineError::Internal {
            detail: format!("cdp layer failure: {other}"),
        },
    }
}

/// Builds a CDP mouse event from the protocol-neutral input event.
pub fn mouse_params(
    event_type: DispatchMouseEventType,
    x: f64,
    y: f64,
) -> DispatchMouseEventParams {
    DispatchMouseEventParams::new(event_type, x, y)
}

/// Maps a protocol-neutral button onto the CDP button.
pub fn cdp_button(button: MouseButton) -> CdpMouseButton {
    match button {
        MouseButton::Left => CdpMouseButton::Left,
        MouseButton::Middle => CdpMouseButton::Middle,
        MouseButton::Right => CdpMouseButton::Right,
    }
}

/// Builds a CDP key event for a named key. Virtual key codes are not
/// derived in v1; sites that require them need a code table (deferred
/// to the session layer's keyboard work).
pub fn key_params(event_type: DispatchKeyEventType, key: &str) -> DispatchKeyEventParams {
    let mut params = DispatchKeyEventParams::new(event_type);
    params.key = Some(key.to_owned());
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_folds_to_engine_timeout() {
        let folded = fold(CdpError::Timeout);
        assert!(matches!(folded, EngineError::Timeout { .. }));
    }

    #[test]
    fn launch_failures_stay_launch_failures() {
        let folded = fold(CdpError::NoResponse);
        assert!(matches!(folded, EngineError::Terminated));
    }

    #[test]
    fn buttons_map_one_to_one() {
        assert!(matches!(
            cdp_button(MouseButton::Left),
            CdpMouseButton::Left
        ));
        assert!(matches!(
            cdp_button(MouseButton::Right),
            CdpMouseButton::Right
        ));
    }

    #[test]
    fn key_params_carry_the_key_name() {
        let params = key_params(DispatchKeyEventType::KeyDown, "Enter");
        assert_eq!(params.key.as_deref(), Some("Enter"));
    }
}
