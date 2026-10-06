//! CDP implementation of page-scoped operations.
//!
//! Boundary: one CDP target per handle. Every operation carries a
//! timeout; navigation uses the context's configured
//! budget, the others a fixed bound so no call can wait forever.
//! Chromiumoxide types are implementation details and never appear in
//! the public `rutter-engine` trait signatures.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::emulation::{
    ScreenOrientation, ScreenOrientationType, SetDeviceMetricsOverrideParams,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventType, DispatchMouseEventType, InsertTextParams,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EventLoadingFailed, EventRequestWillBeSent, EventResponseReceived, RequestId,
};
use chromiumoxide::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, EventFrameNavigated,
    EventJavascriptDialogOpening, EventScreencastFrame, GetFrameTreeParams,
    GetNavigationHistoryParams, HandleJavaScriptDialogParams, NavigateParams,
    NavigateToHistoryEntryParams, ScreencastFrameAckParams, StartScreencastFormat,
    StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::cdp::js_protocol::runtime::{
    CallFunctionOnParams, EvaluateParams, EventConsoleApiCalled, EventExceptionThrown,
    ExecutionContextId, GetPropertiesParams, RemoteObject, RemoteObjectType,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::listeners::EventStream;
use chromiumoxide::page::ScreenshotParams;
use serde_json::Value;

use rutter_engine::error::EngineError;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{
    ConsoleLevel, DialogKind, ImageFormat, ObservationStream, PageObservation, RequestEntry,
    ScreencastFrame, ScreencastStream, Screenshot,
};
use std::collections::{HashMap, VecDeque};

use crate::error::{self, COMMAND_TIMEOUT, fold, fold_navigation, with_deadline, with_deadline_by};
use crate::keys;

/// Budget for the page's URL query. The query is an untyped oneshot
/// into the chromiumoxide handler: if the handler died (browser crash
/// mid-navigation), the send fails, but a wedged handler would hang a
/// bare await forever, so it gets its own deadline like every call.
const URL_TIMEOUT: Duration = Duration::from_secs(10);

/// Screencast capture parameters, shared by the initial start and the
/// post-navigation restart: the two must stay identical so the viewer
/// sees one continuous stream across navigations.
fn screencast_params() -> StartScreencastParams {
    StartScreencastParams::builder()
        .format(StartScreencastFormat::Jpeg)
        .quality(50)
        .max_width(1024)
        .every_nth_frame(1)
        .build()
}

/// Cap on one observation's text: a hostile or chatty page must not be
/// able to bloat the event ring or a console feed with one call.
const OBSERVATION_TEXT_CAP: usize = 2_000;

/// The world a scope is minted in when the document is already loaded.
/// Named rather than anonymous so a repeated call reuses it.
const ISOLATED_WORLD: &str = "rutter-ref-scope";

/// The global the serializer reads its scope from; `entropy.js` locks the
/// same name onto the global at document start.
pub(crate) const REF_SCOPE_PROPERTY: &str = "__rutterRefScope";

/// The isolated-world marker that binds a defined scope to the document
/// it was defined on. The page cannot reach the isolated world, and a
/// navigation replaces the world's global, so a claim read back from it
/// proves the locked minter answering that scope is the one this engine
/// defined on *this* document — not one a later document planted after
/// learning the scope from its own minter, which is callable from page
/// script by design.
const CLAIM_PROPERTY: &str = "__rutterEngineClaim";

/// The prefix of the one define error the browser produced itself: the
/// evaluation ran and the property refused the install. Every other
/// define failure -- a timeout, a transport drop -- leaves whether the
/// define executed unknown, and preparation records its candidate for
/// a later verification instead of assuming the attempt never landed.
pub(crate) const DEFINE_REFUSED_PREFIX: &str = "install the document scope:";

/// How many base36 characters a scope carries, as the snapshot format
/// documents: 36^6 puts a two-document collision past two billion to
/// one, far outside any session's reach.
const REF_SCOPE_CHARS: usize = 6;

/// Mints one scope in an isolated world, where `crypto` is the
/// platform's own and no page patch has reached it. Six bytes are 48
/// bits, inside the integer range a Number carries exactly.
const SCOPE_IN_ISOLATED_WORLD: &str = "(function () { \
   var bytes = new Uint8Array(6); \
   crypto.getRandomValues(bytes); \
   var value = 0; \
   for (var index = 0; index < bytes.length; index += 1) { value = value * 256 + bytes[index]; } \
   var digits = '0123456789abcdefghijklmnopqrstuvwxyz'; \
   var scope = ''; \
   for (var index = 0; index < 6; index += 1) { \
     var digit = value % 36; \
     scope = digits[digit] + scope; \
     value = (value - digit) / 36; \
   } \
   return scope; \
 })()";

/// Whether a value is a scope the snapshot format accepts: exactly
/// [`REF_SCOPE_CHARS`] base36 characters. Checked before the value is
/// spliced into a script, so a malformed one cannot become script text.
fn is_scope(scope: &str) -> bool {
    scope.len() == REF_SCOPE_CHARS
        && scope
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

/// Bounds an observation text at [`OBSERVATION_TEXT_CAP`] characters.
fn cap_text(text: String) -> String {
    // Byte length bounds the char count, so a short line — the common
    // console entry and request URL — skips the full character scan.
    if text.len() <= OBSERVATION_TEXT_CAP || text.chars().count() <= OBSERVATION_TEXT_CAP {
        return text;
    }
    let mut capped: String = text.chars().take(OBSERVATION_TEXT_CAP).collect();
    capped.push('…');
    capped
}

/// Names a request field the observation feed never learned. Events can
/// arrive for requests that started before the feed attached, and the
/// failure event carries no URL of its own; rendering the empty string
/// would show `-> failed` with blank fields instead of saying why.
fn or_unknown(field: String) -> String {
    if field.is_empty() {
        "<unknown>".to_owned()
    } else {
        field
    }
}

/// Maps a protocol dialog kind onto the engine vocabulary.
fn dialog_kind(kind: &str) -> DialogKind {
    match kind {
        "confirm" => DialogKind::Confirm,
        "prompt" => DialogKind::Prompt,
        "beforeunload" => DialogKind::Beforeunload,
        _ => DialogKind::Alert,
    }
}

/// Maps a protocol console call type onto the engine severity; the
/// grouped and structured calls count as ordinary log output.
fn console_level(call_type: &str) -> ConsoleLevel {
    match call_type {
        "debug" => ConsoleLevel::Debug,
        "info" => ConsoleLevel::Info,
        "warning" => ConsoleLevel::Warning,
        "error" | "assert" => ConsoleLevel::Error,
        _ => ConsoleLevel::Log,
    }
}

/// Formats one console argument the way a developer console would: a
/// string argument verbatim, other values as JSON, and a type-name
/// placeholder when the value carried neither.
fn console_argument(argument: &RemoteObject) -> String {
    if let Some(value) = argument.value.as_ref() {
        return match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
    }
    if let Some(description) = argument.description.as_deref() {
        return description.to_owned();
    }
    format!("[{}]", argument.r#type.as_ref())
}

/// Formats a console call's text from its arguments, joined with
/// spaces like a developer console's default formatting.
fn console_text(arguments: &[RemoteObject]) -> String {
    let joined = arguments
        .iter()
        .map(console_argument)
        .collect::<Vec<_>>()
        .join(" ");
    cap_text(joined)
}

/// Formats an uncaught exception's text: the thrown value's
/// description when the protocol carried one, otherwise the bare
/// exception text.
fn exception_text(details: &chromiumoxide::cdp::js_protocol::runtime::ExceptionDetails) -> String {
    let text = details
        .exception
        .as_ref()
        .and_then(|exception| exception.description.as_deref())
        .unwrap_or(details.text.as_str());
    cap_text(text.to_owned())
}

/// Resolves the instant the next capture may run and records it as the
/// new reservation, given the previous one. Pure so the spacing rules
/// are unit-testable without a page.
fn capture_due(last: Option<Instant>, min_interval: Duration, now: Instant) -> Instant {
    let wait = match last {
        // A future `at` is a slot a concurrent capture already reserved;
        // `elapsed()` saturates to zero there, which would shorten the
        // spacing, so this capture queues behind that slot instead.
        Some(at) if at > now => (at - now) + min_interval,
        // Both arms measure against the caller's `now`, keeping the
        // function deterministic (a second clock read inside would race
        // the first).
        Some(at) => min_interval.saturating_sub(now - at),
        None => Duration::ZERO,
    };
    now + wait
}

/// Takes the page's observation claim, refusing a second consumer. The
/// claim is taken before the listeners are registered so a racing second
/// caller cannot slip between; the mutex guard never spans an await.
fn claim_observation(claimed: &Mutex<bool>) -> Result<(), EngineError> {
    let mut claimed = claimed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *claimed {
        return Err(EngineError::Unsupported {
            operation: "observe".to_owned(),
            reason: "this page's observation feed already has a consumer".to_owned(),
        });
    }
    *claimed = true;
    Ok(())
}

/// Releases an observation claim taken but never handed out: a listener
/// registration that failed must not leave the flag set, or the page's
/// feed stays permanently unusable with no consumer at all.
fn release_observation_claim(claimed: &Mutex<bool>) {
    *claimed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = false;
}

/// One CDP target wrapped as a page handle.
pub struct CdpPage {
    page: Page,
    navigation_timeout: Duration,
    /// Minimum interval between captures, from the context caps; `None`
    /// disables the rate limit.
    screenshot_min_interval: Option<Duration>,
    last_capture: Mutex<Option<Instant>>,
    /// Whether the page's observation feed has been handed out; the
    /// feed is single-consumer, so a second `observe` call fails
    /// instead of silently splitting the observations.
    observation_claimed: Mutex<bool>,
    /// Whether closing this page may close the underlying target.
    /// Pages attached to a target that already existed (the Electron
    /// engine's one visible surface, created by its own UI) outlive
    /// their handle: closing that target would take the browser's
    /// window content with it.
    closable: bool,
}

impl CdpPage {
    /// Creates a handle over a chromiumoxide page.
    pub fn new(
        page: Page,
        navigation_timeout: Duration,
        screenshot_min_interval: Option<Duration>,
    ) -> Self {
        Self {
            page,
            navigation_timeout,
            screenshot_min_interval,
            last_capture: Mutex::new(None),
            observation_claimed: Mutex::new(false),
            closable: true,
        }
    }

    /// Wraps a target that the browser owned before this handle asked
    /// for a page. The target belongs to the engine's own UI, so it
    /// survives the handle: `close_page` removes the registration only.
    pub fn attach(
        page: Page,
        navigation_timeout: Duration,
        screenshot_min_interval: Option<Duration>,
    ) -> Self {
        Self::adopted(page, navigation_timeout, screenshot_min_interval, false)
    }

    /// Wraps a target this context did not create. `closable` marks the
    /// rare case where the target still sits inside the adopting
    /// context — a `window.open` tab of the session's own isolation —
    /// so closing it is legitimate; every engine-owned surface (the app
    /// window, shells without context support) stays unclosable.
    pub fn adopted(
        page: Page,
        navigation_timeout: Duration,
        screenshot_min_interval: Option<Duration>,
        closable: bool,
    ) -> Self {
        Self {
            page,
            navigation_timeout,
            screenshot_min_interval,
            last_capture: Mutex::new(None),
            observation_claimed: Mutex::new(false),
            closable,
        }
    }

    /// Whether closing this page may close the underlying target.
    pub fn closable(&self) -> bool {
        self.closable
    }

    /// Registers the entropy capture for every document this page loads
    /// from now on, in the main world and in the named isolated world.
    ///
    /// A document's ref scope keeps a stale reference captured on one
    /// page from resolving to another page's element after a tab switch,
    /// and it has to be a value the document cannot choose: every global
    /// a page can reach -- `Math.random`, `crypto`, the prototypes a
    /// conversion would go through -- is one it can replace. The capture
    /// script runs before the document's own scripts and holds
    /// everything it needs from the platform; see
    /// [`rutter_observe::entropy_capture_script`].
    ///
    /// The shadow copy in the isolated world defines the same minter
    /// where no page script can reach. It never runs in the serializer;
    /// its presence is what [`CdpPage::document_start_ran`] reads as the
    /// proof that the document-start registration covered the document
    /// that is now current, so the locked property in the main world was
    /// installed by the engine rather than by the page. Both worlds draw
    /// from the same platform `crypto`, so the shadow succeeding while
    /// the main-world capture failed is not a case: the two
    /// registrations are installed back to back and run back to back
    /// before any page script.
    ///
    /// Registration is per CDP session and per target and survives
    /// navigation, so once per session is enough for every document the
    /// page loads afterwards, including ones the page starts itself. A
    /// handle re-prepared on its session adds the registrations again;
    /// the script's own locked define turns the repeat into a swallowed
    /// throw. A failure is reported rather than swallowed: a page whose
    /// scope the page itself can choose is a weaker guarantee than the
    /// one callers are promised. Every call is under a deadline like the
    /// rest of this file, so a browser that stops answering cannot park
    /// a page open.
    pub async fn install_entropy_capture(&self) -> Result<(), EngineError> {
        let source = rutter_observe::entropy_capture_script();
        let params = AddScriptToEvaluateOnNewDocumentParams::builder()
            .source(source)
            .build()
            .map_err(|error| EngineError::Internal {
                detail: format!("build the entropy capture registration: {error}"),
            })?;
        with_deadline(
            "install_entropy_capture",
            COMMAND_TIMEOUT,
            self.page.execute(params),
        )
        .await?;
        let shadow = AddScriptToEvaluateOnNewDocumentParams::builder()
            .source(source)
            .world_name(ISOLATED_WORLD.to_owned())
            .build()
            .map_err(|error| EngineError::Internal {
                detail: format!("build the entropy shadow registration: {error}"),
            })?;
        with_deadline(
            "install_entropy_capture_shadow",
            COMMAND_TIMEOUT,
            self.page.execute(shadow),
        )
        .await?;
        Ok(())
    }

    /// Whether the document-start registration covered the document that
    /// is current: the named isolated world carries the minter the
    /// registration defines there.
    ///
    /// The page cannot reach an isolated world, so this answer cannot be
    /// forged from page script. A minter in the named world means the
    /// engine's document-start script ran for this document before the
    /// page's own, and the locked property the serializer reads in the
    /// main world is therefore the engine's, however hostile the page --
    /// which is what lets preparation accept an installed minter instead
    /// of defining over it, a define that would throw.
    ///
    /// Every call is under a deadline like the rest of this file: this
    /// runs while a page is being opened or adopted, so a browser that
    /// stops answering must not park that.
    pub(crate) async fn document_start_ran(&self) -> Result<bool, EngineError> {
        let context_id = self.isolated_world().await?;
        let probe = with_deadline(
            "probe_document_start",
            COMMAND_TIMEOUT,
            self.page.execute(
                EvaluateParams::builder()
                    .expression(format!("typeof {REF_SCOPE_PROPERTY} === 'function'"))
                    .context_id(context_id)
                    .return_by_value(true)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the document-start probe: {error}"),
                    })?,
            ),
        )
        .await?;
        if let Some(details) = probe.result.exception_details {
            return Err(EngineError::Internal {
                detail: format!(
                    "probe the document-start registration: {}",
                    exception_text(&details)
                ),
            });
        }
        Ok(probe
            .result
            .result
            .value
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// The execution context of the named isolated world, creating the
    /// world for the frame when absent; a repeated call reuses it.
    async fn isolated_world(&self) -> Result<ExecutionContextId, EngineError> {
        let tree = with_deadline(
            "get_frame_tree",
            COMMAND_TIMEOUT,
            self.page.execute(GetFrameTreeParams::default()),
        )
        .await?;
        let world = with_deadline(
            "create_isolated_world",
            COMMAND_TIMEOUT,
            self.page.execute(
                CreateIsolatedWorldParams::builder()
                    .frame_id(tree.frame_tree.frame.id.clone())
                    .world_name(ISOLATED_WORLD)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the isolated world request: {error}"),
                    })?,
            ),
        )
        .await?;
        Ok(world.execution_context_id)
    }

    /// Records `scope` as this document's engine claim in the isolated
    /// world, overwriting any claim an earlier attempt left.
    ///
    /// A plain binding, not a locked define: the world is unreachable
    /// from page script, so there is nothing to lock against, and the
    /// overwrite is what keeps a retry consistent — a define that timed
    /// out ambiguously leaves the claim answering the candidate, which
    /// the retry then verifies against the live minter. The claim dies
    /// with the document: a navigation replaces the world's global, so
    /// a remembered scope can never verify against a later document,
    /// however the page acquired it.
    pub(crate) async fn claim_document_scope(&self, scope: &str) -> Result<(), EngineError> {
        let context_id = self.isolated_world().await?;
        let claim = with_deadline(
            "claim_document_scope",
            COMMAND_TIMEOUT,
            self.page.execute(
                EvaluateParams::builder()
                    .expression(format!("{CLAIM_PROPERTY} = '{scope}'"))
                    .context_id(context_id)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the scope claim: {error}"),
                    })?,
            ),
        )
        .await?;
        if let Some(details) = claim.result.exception_details {
            return Err(EngineError::Internal {
                detail: format!("claim the document scope: {}", exception_text(&details)),
            });
        }
        Ok(())
    }

    /// Whether this document carries the engine claim for `scope`.
    ///
    /// The marker was written by [`Self::claim_document_scope`] right
    /// before the define it names, so a match proves the locked minter
    /// answering `scope` was installed by this engine *on this
    /// document* — the proof a same-document retry needs, and the
    /// reason a page that replants a learned scope on a fresh document
    /// fails instead.
    pub(crate) async fn document_scope_claimed(&self, scope: &str) -> Result<bool, EngineError> {
        let context_id = self.isolated_world().await?;
        let probe = with_deadline(
            "probe_scope_claim",
            COMMAND_TIMEOUT,
            self.page.execute(
                EvaluateParams::builder()
                    .expression(format!("{CLAIM_PROPERTY} === '{scope}'"))
                    .context_id(context_id)
                    .return_by_value(true)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the claim probe: {error}"),
                    })?,
            ),
        )
        .await?;
        if let Some(details) = probe.result.exception_details {
            return Err(EngineError::Internal {
                detail: format!("probe the scope claim: {}", exception_text(&details)),
            });
        }
        Ok(probe
            .result
            .result
            .value
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// Draws one scope in an isolated world: a separate realm with the
    /// platform's own `crypto`, which the page's patches do not reach.
    pub(crate) async fn mint_scope_in_isolated_world(&self) -> Result<String, EngineError> {
        let context_id = self.isolated_world().await?;
        let minted = with_deadline(
            "mint_ref_scope",
            COMMAND_TIMEOUT,
            self.page.execute(
                EvaluateParams::builder()
                    .expression(SCOPE_IN_ISOLATED_WORLD)
                    .context_id(context_id)
                    .return_by_value(true)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the scope evaluation: {error}"),
                    })?,
            ),
        )
        .await?;
        if let Some(details) = minted.result.exception_details {
            return Err(EngineError::Internal {
                detail: format!("mint the document scope: {}", exception_text(&details)),
            });
        }
        minted
            .result
            .result
            .value
            .as_ref()
            .and_then(Value::as_str)
            .filter(|scope| is_scope(scope))
            .map(str::to_owned)
            .ok_or_else(|| EngineError::Internal {
                detail: "the isolated world returned no usable scope".to_owned(),
            })
    }

    /// Defines the scope in the main world as a locked property.
    ///
    /// The expression is wrapped so the evaluation returns nothing:
    /// `Object.defineProperty` answers with the object it was given, and
    /// asking CDP to serialize the window back is an error, not a result.
    /// An exception here means the property already exists as a
    /// non-configurable one the caller has not verified as its own --
    /// the page planted it, or an earlier install of this engine did and
    /// the recorded scope must be checked against it instead.
    pub(crate) async fn define_locked_scope(&self, scope: &str) -> Result<(), EngineError> {
        let install = format!(
            "(function () {{ Object.defineProperty(window, '{REF_SCOPE_PROPERTY}', {{ \
               value: function () {{ return '{scope}'; }}, \
               writable: false, configurable: false, enumerable: false }}); }})()"
        );
        let installed = with_deadline(
            "install_ref_scope",
            COMMAND_TIMEOUT,
            self.page.execute(
                EvaluateParams::builder()
                    .expression(install)
                    .return_by_value(true)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the scope installation: {error}"),
                    })?,
            ),
        )
        .await?;
        match installed.result.exception_details {
            Some(details) => Err(EngineError::Internal {
                detail: format!("{DEFINE_REFUSED_PREFIX} {}", exception_text(&details)),
            }),
            None => Ok(()),
        }
    }

    /// Checks the installation through CDP rather than by reading the
    /// property back in the page.
    ///
    /// The expression above runs in the main world, where a page can
    /// replace `Object.defineProperty` with a no-op -- or
    /// `Object.getOwnPropertyDescriptor` with something that lies -- and
    /// either would leave the snapshot minting from a scope the page
    /// chose. The browser's own view of the global cannot be forged from
    /// inside the page, so the property has to be an own, non-writable,
    /// non-configurable function *and* that function has to answer with
    /// the scope that was minted: a page that kept its own locked
    /// function in place returns its own value, and the comparison
    /// catches it. The same check against a remembered scope is how a
    /// retry recognizes an install of its own.
    ///
    /// `Ok(true)` means the minter the browser reports answers with the
    /// scope named; `Ok(false)` means the browser answered and the
    /// minter is someone else's; `Err` means nothing was learned -- a
    /// wedged or slow browser must fail the caller rather than hand
    /// back a page whose minter is unconfirmed.
    ///
    /// The boundary this check cannot cross: a document that was already
    /// loaded when rutter attached can have made `Object.defineProperty`
    /// a spying no-op, so the define lands nowhere and the answer is the
    /// page's echo of the scope it saw. That page owns its realm either
    /// way — the labels it renders are equally its choice — and every
    /// document the registration covers is immune, because the
    /// document-start install runs before any page script.
    pub(crate) async fn verify_locked_scope(&self, scope: &str) -> Result<bool, EngineError> {
        let minter = self.locked_scope_minter().await?;
        let object_id = minter
            .object_id
            .clone()
            .ok_or_else(|| EngineError::Internal {
                detail: format!("the browser returned no handle for {REF_SCOPE_PROPERTY}"),
            })?;
        // Called on the function object the browser itself reported, so
        // the call cannot land on anything the page swapped in after the
        // check.
        let answered = with_deadline(
            "call_ref_scope",
            COMMAND_TIMEOUT,
            self.page.execute(
                CallFunctionOnParams::builder()
                    .object_id(object_id)
                    .function_declaration("function () { return this(); }")
                    .return_by_value(true)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the minter call: {error}"),
                    })?,
            ),
        )
        .await?;
        if let Some(details) = answered.result.exception_details {
            return Err(EngineError::Internal {
                detail: format!("call the document scope: {}", exception_text(&details)),
            });
        }
        let answered = answered
            .result
            .result
            .value
            .as_ref()
            .and_then(Value::as_str);
        Ok(answered == Some(scope))
    }

    /// The locked scope minter the document holds, as the browser sees it.
    ///
    /// A property that is missing, writable, configurable, or not a
    /// function is not the one this engine installed.
    ///
    /// The global is read through `this` — a sloppy function's `this` is
    /// the realm's global object itself, a binding the page cannot
    /// reassign — and not through `globalThis` or `window`, both of
    /// which are properties a pre-loaded page can point at a look-alike
    /// before rutter attaches.
    pub(crate) async fn locked_scope_minter(&self) -> Result<RemoteObject, EngineError> {
        let global = with_deadline(
            "read_global",
            COMMAND_TIMEOUT,
            self.page.execute(
                EvaluateParams::builder()
                    .expression("(function () { return this; })()")
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the global read: {error}"),
                    })?,
            ),
        )
        .await?;
        let object_id =
            global
                .result
                .result
                .object_id
                .clone()
                .ok_or_else(|| EngineError::Internal {
                    detail: "the browser returned no handle for the global object".to_owned(),
                })?;
        let properties = with_deadline(
            "read_global_properties",
            COMMAND_TIMEOUT,
            self.page.execute(
                GetPropertiesParams::builder()
                    .object_id(object_id)
                    .own_properties(true)
                    .build()
                    .map_err(|error| EngineError::Internal {
                        detail: format!("build the property read: {error}"),
                    })?,
            ),
        )
        .await?;
        properties
            .result
            .result
            .iter()
            .find(|property| property.name == REF_SCOPE_PROPERTY)
            .filter(|property| {
                property.writable == Some(false)
                    && !property.configurable
                    && property
                        .value
                        .as_ref()
                        .is_some_and(|value| value.r#type == RemoteObjectType::Function)
            })
            .and_then(|property| property.value.clone())
            .ok_or_else(|| EngineError::Internal {
                detail: format!(
                    "the document did not take a locked {REF_SCOPE_PROPERTY}: \
                     a page that can replace it can choose its own scope"
                ),
            })
    }

    /// Target id for closing the tab through the browser connection
    /// (avoids consuming the page handle).
    pub fn target_id(&self) -> TargetId {
        self.page.target_id().clone()
    }

    /// Reads the page URL under a deadline; see [`URL_TIMEOUT`] for why
    /// this query cannot go bare.
    async fn url(&self) -> Result<Option<String>, EngineError> {
        with_deadline("page_url", URL_TIMEOUT, self.page.url()).await
    }

    /// Walks the session history by `offset` entries and resolves with
    /// the effective URL after the navigation settles.
    async fn navigate_history(&self, offset: i64) -> Result<String, EngineError> {
        with_deadline("history", self.navigation_timeout, async {
            let history = self
                .page
                .execute(GetNavigationHistoryParams::default())
                .await?;
            let target = history.result.current_index as i64 + offset;
            if target < 0 || target >= history.result.entries.len() as i64 {
                return Err(CdpError::ChromeMessage(format!(
                    "no history entry at offset {offset}"
                )));
            }
            let entry_id = history.result.entries[target as usize].id;
            self.page
                .execute(NavigateToHistoryEntryParams::new(entry_id))
                .await?;
            self.page.wait_for_navigation().await
        })
        .await?;

        self.url().await?.ok_or_else(|| EngineError::Internal {
            detail: "page URL unavailable after history navigation".to_owned(),
        })
    }

    /// The event streams the page's observation feed consumes.
    ///
    /// An inherent method rather than part of `PageHandle`: the streams
    /// are chromiumoxide's, and no other engine backend names them.
    ///
    /// Registration is all-or-nothing in effect: `observe` releases the
    /// claim when this fails, because nothing was handed out and a set
    /// claim would only disable the page's feed for the rest of its
    /// life.
    async fn observation_listeners(&self) -> Result<ObservationListeners, EngineError> {
        let dialogs = self
            .page
            .event_listener::<EventJavascriptDialogOpening>()
            .await
            .map_err(fold)?;
        let console = self
            .page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .map_err(fold)?;
        let exceptions = self
            .page
            .event_listener::<EventExceptionThrown>()
            .await
            .map_err(fold)?;
        let requests = self
            .page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(fold)?;
        let responses = self
            .page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(fold)?;
        let failures = self
            .page
            .event_listener::<EventLoadingFailed>()
            .await
            .map_err(fold)?;
        Ok((dialogs, console, exceptions, requests, responses, failures))
    }
}

#[async_trait]
impl rutter_engine::page::PageHandle for CdpPage {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        with_deadline_by(
            "navigate",
            self.navigation_timeout,
            async {
                self.page.goto(NavigateParams::new(url)).await?;
                self.page.wait_for_navigation().await
            },
            |error| fold_navigation(error, url),
        )
        .await?;

        self.url().await?.ok_or_else(|| EngineError::Internal {
            detail: "page URL unavailable after navigation".to_owned(),
        })
    }

    async fn reload(&self) -> Result<(), EngineError> {
        with_deadline("reload", self.navigation_timeout, self.page.reload()).await?;
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        self.navigate_history(-1).await
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        self.navigate_history(1).await
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        let mut params = EvaluateParams::new(expression);
        params.return_by_value = Some(true);
        let result = with_deadline(
            "evaluate",
            COMMAND_TIMEOUT,
            self.page.evaluate_expression(params),
        )
        .await?;
        Ok(result.value().cloned().unwrap_or(Value::Null))
    }

    async fn dispatch_input(&self, event: InputEvent) -> Result<(), EngineError> {
        let command = match event {
            InputEvent::MouseMove { x, y } => {
                error::mouse_params(DispatchMouseEventType::MouseMoved, x, y)
            }
            InputEvent::MousePressed { x, y, button } => {
                let mut params = error::mouse_params(DispatchMouseEventType::MousePressed, x, y);
                params.button = Some(error::cdp_button(button));
                params.click_count = Some(1);
                params
            }
            InputEvent::MouseReleased { x, y, button } => {
                let mut params = error::mouse_params(DispatchMouseEventType::MouseReleased, x, y);
                params.button = Some(error::cdp_button(button));
                params.click_count = Some(1);
                params
            }
            InputEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                let mut params = error::mouse_params(DispatchMouseEventType::MouseWheel, x, y);
                params.delta_x = Some(delta_x);
                params.delta_y = Some(delta_y);
                params
            }
            InputEvent::InsertText { text } => {
                let params = InsertTextParams::new(text);
                return with_deadline("insert_text", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
            InputEvent::KeyPressed { key } => {
                let params = keys::key_params(DispatchKeyEventType::KeyDown, &key);
                return with_deadline("dispatch_key", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
            InputEvent::KeyReleased { key } => {
                let params = keys::key_params(DispatchKeyEventType::KeyUp, &key);
                return with_deadline("dispatch_key", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
        };
        with_deadline(
            "dispatch_mouse",
            COMMAND_TIMEOUT,
            self.page.execute(command),
        )
        .await
        .map(|_| ())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        // Enforce the context's capture-rate cap so one caller cannot
        // flood the engine with captures.
        if let Some(min_interval) = self.screenshot_min_interval {
            let now = Instant::now();
            let due = {
                let mut last = self
                    .last_capture
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let due = capture_due(*last, min_interval, now);
                *last = Some(due);
                due
            };
            if due > now {
                tokio::time::sleep(due - now).await;
            }
        }

        let data = with_deadline(
            "screenshot",
            COMMAND_TIMEOUT,
            self.page.screenshot(ScreenshotParams::default()),
        )
        .await?;
        Ok(Screenshot {
            format: ImageFormat::Png,
            data,
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        use futures::StreamExt;

        // Register the listeners before starting so early frames are
        // not lost (correct ack loop).
        let mut frames = self
            .page
            .event_listener::<EventScreencastFrame>()
            .await
            .map_err(fold)?;
        let mut navigations = self
            .page
            .event_listener::<EventFrameNavigated>()
            .await
            .map_err(fold)?;
        let start = screencast_params();
        with_deadline(
            "start_screencast",
            COMMAND_TIMEOUT,
            self.page.execute(start),
        )
        .await?;

        let (sender, receiver) = tokio::sync::mpsc::channel::<ScreencastFrame>(4);
        let task_page = self.page.clone();
        // The forwarding task owns the capture lifecycle: acks every
        // frame, restarts after navigations where CDP
        // stops the capture on its own, and stops when the viewer drops
        // the stream. Every CDP call inside stays under a deadline so a
        // dead browser ends the stream instead of parking the task.
        //
        // Frames are handed off without awaiting: a stalled viewer must
        // never block this task, because the backlog would then pile up
        // in the chromiumoxide event listener, which is an unbounded
        // queue. Frames are droppable (latest-wins backpressure); over
        // a full channel the newest frames are
        // dropped and the memory stays bounded by the channel capacity.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(event) = frames.next() => {
                        let _ = with_deadline(
                            "screencast_ack",
                            COMMAND_TIMEOUT,
                            task_page.execute(ScreencastFrameAckParams::new(event.session_id)),
                        )
                        .await;
                        // A slot is reserved before the decode so a
                        // stalled viewer drops frames at the cheap end:
                        // over a full channel the frame is discarded
                        // without paying the base64 decode, and the
                        // memory stays bounded by the channel capacity.
                        // Acks are unchanged — one per frame, sent
                        // before this decision either way — so CDP's
                        // capture cadence and the latest-wins delivery
                        // are what they were.
                        match sender.try_reserve() {
                            Ok(permit) => {
                                use base64::Engine as _;
                                // On a decode failure the permit drops
                                // and the slot frees; the ack above was
                                // already sent, exactly as before.
                                if let Ok(jpeg) = base64::engine::general_purpose::STANDARD
                                    .decode(AsRef::<[u8]>::as_ref(&event.data))
                                {
                                    permit.send(ScreencastFrame { jpeg });
                                }
                            }
                            // A slow viewer loses frames, not memory.
                            Err(tokio::sync::mpsc::error::TrySendError::Full(())) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => break,
                        }
                    }
                    Some(_) = navigations.next() => {
                        // CDP stops the capture on navigation; restart it.
                        let _ = with_deadline(
                            "screencast_restart",
                            COMMAND_TIMEOUT,
                            task_page.execute(screencast_params()),
                        )
                        .await;
                    }
                    // A quiet page paints no frames, so the Closed arm
                    // above may not fire for a long time; watching the
                    // receiver directly ends the capture the moment the
                    // viewer leaves (streaming stops when the last
                    // viewer leaves).
                    _ = sender.closed() => break,
                    else => break,
                }
            }
            let _ = with_deadline(
                "stop_screencast",
                COMMAND_TIMEOUT,
                task_page.execute(StopScreencastParams::default()),
            )
            .await;
        });

        Ok(ScreencastStream::new(receiver))
    }

    async fn observe(&self) -> Result<ObservationStream, EngineError> {
        claim_observation(&self.observation_claimed)?;

        // Register the listeners before returning so observations from
        // the first ticks are not lost.
        let listeners = match self.observation_listeners().await {
            Ok(listeners) => listeners,
            Err(error) => {
                release_observation_claim(&self.observation_claimed);
                return Err(error);
            }
        };

        let (sender, receiver) = tokio::sync::mpsc::channel::<PageObservation>(64);
        // The forwarding task owns the feed: observations hand off
        // without awaiting, so a stalled consumer loses observations
        // instead of piling backlog into the chromiumoxide listeners,
        // which are an unbounded queue (same rule as the screencast).
        // Every observation is droppable; the feed ends when the page
        // or the consumer goes away.
        tokio::spawn(forward_observations(listeners, sender));

        Ok(ObservationStream::new(receiver))
    }

    async fn handle_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), EngineError> {
        let mut params = HandleJavaScriptDialogParams::new(accept);
        params.prompt_text = prompt_text.map(str::to_owned);
        with_deadline("handle_dialog", COMMAND_TIMEOUT, self.page.execute(params))
            .await
            .map(|_| ())
    }

    async fn set_input_files(&self, reference: &str, files: &[String]) -> Result<(), EngineError> {
        // The element resolves through the same page-side reference
        // store every snapshot script uses (rutter-observe owns that
        // contract); the element comes back as a remote object handle,
        // which the file operation targets. A stale reference answers
        // `null` and carries no object id.
        let mut params = EvaluateParams::new(rutter_observe::element_script(reference));
        params.return_by_value = Some(false);
        let result = with_deadline("set_input_files_resolve", COMMAND_TIMEOUT, async {
            self.page.evaluate_expression(params).await
        })
        .await?;
        let object_id =
            result
                .object()
                .object_id
                .clone()
                .ok_or_else(|| EngineError::ReferenceExpired {
                    reference: reference.to_owned(),
                })?;
        let mut set = SetFileInputFilesParams::new(files.to_vec());
        set.object_id = Some(object_id);
        with_deadline("set_input_files", COMMAND_TIMEOUT, self.page.execute(set))
            .await
            .map(|_| ())
    }

    async fn set_viewport(&self, width: u32, height: u32) -> Result<(), EngineError> {
        let metrics = SetDeviceMetricsOverrideParams::builder()
            .mobile(false)
            .width(width)
            .height(height)
            .device_scale_factor(1.0)
            .screen_orientation(ScreenOrientation::new(
                ScreenOrientationType::PortraitPrimary,
                0,
            ))
            .build()
            .map_err(|error| EngineError::Internal {
                detail: format!("viewport override params: {error}"),
            })?;
        with_deadline("set_viewport", COMMAND_TIMEOUT, self.page.execute(metrics))
            .await
            .map(|_| ())
    }
}

/// Correlates requests that were sent with their terminal outcome.
/// Requests that never finish are bounded: once [`PENDING_CAP`] are
/// in flight, the oldest falls off and its eventual outcome degrades
/// to a method-less entry. Free of I/O so the rules are unit-testable.
#[derive(Default)]
struct PendingRequests {
    map: HashMap<RequestId, (String, String)>,
    order: VecDeque<RequestId>,
}

/// How many requests may sit unanswered before the oldest is dropped.
const PENDING_CAP: usize = 256;

impl PendingRequests {
    fn note(&mut self, id: RequestId, method: String, url: String) {
        if !self.map.contains_key(&id)
            && self.order.len() >= PENDING_CAP
            && let Some(oldest) = self.order.pop_front()
        {
            self.map.remove(&oldest);
        }
        if !self.map.contains_key(&id) {
            self.order.push_back(id.clone());
        }
        self.map.insert(id, (method, url));
    }

    fn take(&mut self, id: &RequestId) -> Option<(String, String)> {
        let entry = self.map.remove(id);
        if entry.is_some() {
            self.order.retain(|pending| pending != id);
        }
        entry
    }
}

/// Hands one observation to the feed's consumer; `false` means the
/// consumer is gone and the forwarding task should end.
fn try_forward(
    sender: &tokio::sync::mpsc::Sender<PageObservation>,
    observation: PageObservation,
) -> bool {
    match sender.try_send(observation) {
        Ok(()) => true,
        // A stalled consumer loses observations, not memory.
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// The CDP event streams one page's observation feed consumes.
///
/// Named as a unit because they are always registered together: a feed
/// missing one of them would silently lose that observation kind, which
/// is why [`Page::observe`] hands the tuple out only when all six
/// registrations succeeded.
type ObservationListeners = (
    EventStream<EventJavascriptDialogOpening>,
    EventStream<EventConsoleApiCalled>,
    EventStream<EventExceptionThrown>,
    EventStream<EventRequestWillBeSent>,
    EventStream<EventResponseReceived>,
    EventStream<EventLoadingFailed>,
);

/// Forwards one page's raw CDP events as observations until the page or
/// the consumer goes away.
///
/// The feed is deliberately lossy: `try_forward` drops what a stalled
/// consumer cannot take instead of awaiting, and the task ends either
/// when the consumer closes the channel or when the page's own events
/// stop arriving.
async fn forward_observations(
    listeners: ObservationListeners,
    sender: tokio::sync::mpsc::Sender<PageObservation>,
) {
    use futures::StreamExt;

    let (mut dialogs, mut console, mut exceptions, mut requests, mut responses, mut failures) =
        listeners;
    // Request outcomes carry the method and URL seen on the way out, so
    // the in-flight map is the forwarder's own state.
    let mut pending = PendingRequests::default();
    loop {
        tokio::select! {
            Some(event) = dialogs.next() => {
                let observation = PageObservation::DialogOpened {
                    kind: dialog_kind(event.r#type.as_ref()),
                    message: cap_text(event.message.clone()),
                };
                if !try_forward(&sender, observation) {
                    break;
                }
            }
            Some(event) = console.next() => {
                let observation = PageObservation::ConsoleEmitted {
                    level: console_level(event.r#type.as_ref()),
                    text: console_text(&event.args),
                };
                if !try_forward(&sender, observation) {
                    break;
                }
            }
            Some(event) = exceptions.next() => {
                let observation = PageObservation::UncaughtException {
                    text: exception_text(&event.exception_details),
                };
                if !try_forward(&sender, observation) {
                    break;
                }
            }
            Some(event) = requests.next() => {
                pending.note(
                    event.request_id.clone(),
                    event.request.method.clone(),
                    event.request.url.clone(),
                );
            }
            Some(event) = responses.next() => {
                let (method, _) = pending
                    .take(&event.request_id)
                    .unwrap_or_else(|| (String::new(), String::new()));
                let observation = PageObservation::RequestObserved {
                    entry: RequestEntry {
                        method: or_unknown(method),
                        url: cap_text(event.response.url.clone()),
                        status: Some(event.response.status.clamp(0, u32::MAX as i64) as u32),
                        resource_type: Some(event.r#type.as_ref().to_lowercase()),
                        error: None,
                    },
                };
                if !try_forward(&sender, observation) {
                    break;
                }
            }
            Some(event) = failures.next() => {
                let (method, url) = pending
                    .take(&event.request_id)
                    .unwrap_or_else(|| (String::new(), String::new()));
                let observation = PageObservation::RequestObserved {
                    entry: RequestEntry {
                        method: or_unknown(method),
                        url: cap_text(or_unknown(url)),
                        status: None,
                        resource_type: Some(event.r#type.as_ref().to_lowercase()),
                        error: Some(cap_text(event.error_text.clone())),
                    },
                };
                if !try_forward(&sender, observation) {
                    break;
                }
            }
            // A quiet page produces no observations, so the Closed arm
            // inside `try_forward` may not fire for a long time; watching
            // the sender directly ends the feed the moment the consumer
            // leaves (same rule as the screencast).
            _ = sender.closed() => break,
            else => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_capture_is_immediate() {
        let now = Instant::now();
        assert_eq!(capture_due(None, Duration::from_secs(10), now), now);
    }

    #[test]
    fn capture_waits_out_the_remaining_interval() {
        let now = Instant::now();
        let last = now - Duration::from_secs(2);
        assert_eq!(
            capture_due(Some(last), Duration::from_secs(10), now),
            now + Duration::from_secs(8)
        );
    }

    #[test]
    fn elapsed_interval_does_not_delay_the_capture() {
        let now = Instant::now();
        let last = now - Duration::from_secs(20);
        assert_eq!(capture_due(Some(last), Duration::from_secs(10), now), now);
    }

    #[test]
    fn concurrent_capture_queues_behind_a_reserved_slot() {
        let now = Instant::now();
        // A slot another capture is already sleeping on; elapsed() would
        // saturate to zero here and reset the schedule instead.
        let reserved = now + Duration::from_secs(7);
        assert_eq!(
            capture_due(Some(reserved), Duration::from_secs(10), now),
            reserved + Duration::from_secs(10)
        );
    }

    #[test]
    fn zero_interval_never_waits() {
        let now = Instant::now();
        assert_eq!(capture_due(Some(now), Duration::ZERO, now), now);
    }

    #[test]
    fn dialog_kinds_map_by_wire_name_with_alert_as_the_default() {
        assert_eq!(dialog_kind("alert"), DialogKind::Alert);
        assert_eq!(dialog_kind("confirm"), DialogKind::Confirm);
        assert_eq!(dialog_kind("prompt"), DialogKind::Prompt);
        assert_eq!(dialog_kind("beforeunload"), DialogKind::Beforeunload);
        assert_eq!(dialog_kind("AccountChooser"), DialogKind::Alert);
    }

    #[test]
    fn console_severities_map_by_call_type() {
        assert_eq!(console_level("log"), ConsoleLevel::Log);
        assert_eq!(console_level("dir"), ConsoleLevel::Log);
        assert_eq!(console_level("table"), ConsoleLevel::Log);
        assert_eq!(console_level("debug"), ConsoleLevel::Debug);
        assert_eq!(console_level("info"), ConsoleLevel::Info);
        assert_eq!(console_level("warning"), ConsoleLevel::Warning);
        assert_eq!(console_level("error"), ConsoleLevel::Error);
        assert_eq!(console_level("assert"), ConsoleLevel::Error);
    }

    #[test]
    fn console_arguments_format_like_a_developer_console() {
        use chromiumoxide::cdp::js_protocol::runtime::{RemoteObject, RemoteObjectType};
        let bare_object = |r#type: RemoteObjectType| RemoteObject {
            r#type,
            subtype: None,
            class_name: None,
            value: None,
            unserializable_value: None,
            description: None,
            deep_serialized_value: None,
            object_id: None,
            preview: None,
            custom_preview: None,
        };
        let string_argument = RemoteObject {
            value: Some(Value::String("careful".to_owned())),
            ..bare_object(RemoteObjectType::String)
        };
        let object_argument = RemoteObject {
            value: Some(serde_json::json!({ "a": 1 })),
            ..bare_object(RemoteObjectType::Object)
        };
        assert_eq!(console_argument(&string_argument), "careful");
        assert_eq!(console_argument(&object_argument), r#"{"a":1}"#);

        let opaque = RemoteObject {
            description: Some("function click() {}".to_owned()),
            ..bare_object(RemoteObjectType::Function)
        };
        assert_eq!(console_argument(&opaque), "function click() {}");

        let bare = bare_object(RemoteObjectType::Symbol);
        assert_eq!(console_argument(&bare), "[symbol]");

        let joined = console_text(&[string_argument, object_argument]);
        assert_eq!(joined, "careful {\"a\":1}");
    }

    #[test]
    fn observation_text_is_capped_for_hostile_pages() {
        let short = cap_text("short".to_owned());
        assert_eq!(short, "short");

        let huge: String = "x".repeat(OBSERVATION_TEXT_CAP * 2);
        let capped = cap_text(huge);
        assert_eq!(capped.chars().count(), OBSERVATION_TEXT_CAP + 1);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn a_feed_consumer_leaving_ends_the_forwarding_decision() {
        let (sender, receiver) = tokio::sync::mpsc::channel::<PageObservation>(1);
        let observation = PageObservation::UncaughtException {
            text: "boom".to_owned(),
        };
        assert!(try_forward(&sender, observation.clone()));
        drop(receiver);
        assert!(
            !try_forward(&sender, observation),
            "a closed feed ends the task"
        );
    }

    #[test]
    fn a_refused_registration_releases_the_claim_for_the_next_attempt() {
        // The claim is taken before the listeners register; a failure in
        // between must hand it back, or the page's feed is dead with no
        // consumer at all.
        let claimed = Mutex::new(false);
        claim_observation(&claimed).expect("a fresh page claims");
        assert!(
            claim_observation(&claimed).is_err(),
            "a second claim is refused"
        );
        release_observation_claim(&claimed);
        claim_observation(&claimed).expect("the released page claims again");
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    fn id(name: &str) -> RequestId {
        RequestId::new(name)
    }

    #[test]
    fn outcomes_pair_with_their_request() {
        let mut pending = PendingRequests::default();
        pending.note(id("r1"), "GET".to_owned(), "https://a.example".to_owned());
        pending.note(id("r2"), "POST".to_owned(), "https://b.example".to_owned());

        assert_eq!(
            pending.take(&id("r1")),
            Some(("GET".to_owned(), "https://a.example".to_owned()))
        );
        assert_eq!(pending.take(&id("r1")), None, "a request answers once");
        assert_eq!(
            pending.take(&id("r2")),
            Some(("POST".to_owned(), "https://b.example".to_owned()))
        );
    }

    #[test]
    fn unknown_requests_degrade_to_a_methodless_entry() {
        let mut pending = PendingRequests::default();
        assert_eq!(pending.take(&id("ghost")), None);
    }

    #[test]
    fn pending_requests_are_bounded_and_the_oldest_falls_off() {
        let mut pending = PendingRequests::default();
        for index in 0..PENDING_CAP + 10 {
            pending.note(id(&format!("r{index}")), "GET".to_owned(), String::new());
        }
        // The oldest ten fell off: their outcomes degrade.
        assert_eq!(pending.take(&id("r0")), None);
        assert_eq!(pending.take(&id("r9")), None);
        assert!(pending.take(&id("r10")).is_some());
        // The cap freed room, so everything answered stays accounted.
        assert_eq!(
            pending.take(&id(&format!("r{}", PENDING_CAP + 9))),
            Some(("GET".to_owned(), String::new()))
        );
    }
}
