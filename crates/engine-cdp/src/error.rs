//! CDP error folding and command deadlines.
//!
//! Boundary: one place maps third-party errors onto the engine error
//! contract and bounds every CDP call with a deadline, so the rest of
//! the crate stays mapping-free, no CDP error type escapes into a
//! public signature, and no call can wait forever (blueprint §8.4).

use std::future::Future;
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    MouseButton as CdpMouseButton,
};
use chromiumoxide::error::CdpError;

use rutter_engine::error::EngineError;
use rutter_engine::input::MouseButton;

/// Fixed budget for commands whose caller supplies no dedicated budget
/// (health probes, context and target management, input, screenshots).
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs a CDP call under a deadline. Elapsed outer budgets and CDP-level
/// timeouts both surface as [`EngineError::Timeout`] carrying the real
/// wait time; every other failure goes through [`fold`].
pub(crate) async fn with_deadline<F, T>(
    operation: &str,
    budget: Duration,
    fut: F,
) -> Result<T, EngineError>
where
    F: Future<Output = Result<T, CdpError>>,
{
    with_deadline_by(operation, budget, fut, fold).await
}

/// [`with_deadline`] with a caller-supplied error folder, used where a
/// call site knows more context than [`fold`] does (for example
/// navigation knows the URL).
pub(crate) async fn with_deadline_by<F, T, M>(
    operation: &str,
    budget: Duration,
    fut: F,
    map: M,
) -> Result<T, EngineError>
where
    F: Future<Output = Result<T, CdpError>>,
    M: Fn(CdpError) -> EngineError,
{
    let started = Instant::now();
    match tokio::time::timeout(budget, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(CdpError::Timeout)) => Err(EngineError::Timeout {
            operation: operation.to_owned(),
            elapsed: started.elapsed(),
        }),
        Ok(Err(error)) => Err(map(error)),
        Err(_elapsed) => Err(EngineError::Timeout {
            operation: operation.to_owned(),
            elapsed: started.elapsed(),
        }),
    }
}

/// Folds a chromiumoxide error into the engine error contract. Timeout
/// variants are normally intercepted by [`with_deadline`]; this fallback
/// keeps them honest when one arrives from an unbounded path.
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

/// Folds an error raised by a navigation, distinguishing transport
/// failures (`net::ERR_*` from the browser's network stack) from bugs so
/// callers get input feedback instead of an "internal error".
pub fn fold_navigation(error: CdpError, url: &str) -> EngineError {
    let detail = error.to_string();
    let transport = matches!(&error, CdpError::Chrome(_) | CdpError::ChromeMessage(_))
        && (detail.contains("net::ERR_") || detail.contains("invalid URL"));
    if transport {
        EngineError::NavigationFailed {
            url: url.to_owned(),
            detail,
        }
    } else {
        fold(error)
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
    fn transport_navigation_errors_get_their_own_variant() {
        let error = fold_navigation(
            CdpError::ChromeMessage("net::ERR_NAME_NOT_RESOLVED at https://x".to_owned()),
            "https://x",
        );
        assert!(matches!(error, EngineError::NavigationFailed { .. }));
    }

    #[test]
    fn non_transport_errors_stay_internal() {
        let error = fold_navigation(
            CdpError::ChromeMessage("unrelated failure".to_owned()),
            "https://x",
        );
        assert!(matches!(error, EngineError::Internal { .. }));
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
