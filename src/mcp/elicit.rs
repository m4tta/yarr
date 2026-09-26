//! High-impact operation elicitation gate (MCP-only).
//!
//! Disruptive and destructive operations get a real,
//! interactive confirmation prompt on the MCP surface via *elicitation* (rmcp
//! [`Peer::elicit_with_timeout`]): before a high-impact action dispatches, the
//! server asks the connected client to confirm, and there is no way to
//! pre-authorize or skip that prompt from the call arguments — the client must
//! actually answer. A client without elicitation capability fails closed; this
//! remote protocol surface never treats missing confirmation as approval.
//!
//! This lives in the MCP protocol layer (not `tools.rs` / the app layer) because
//! elicitation needs the client [`Peer`], exactly like the scope checks in
//! `rmcp_server.rs`. `tools.rs` stays a thin dispatcher.

use std::time::Duration;

use rmcp::{
    RoleServer,
    service::{ElicitationError, Peer},
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Max time to wait for the user to answer a high-impact-operation prompt. On expiry
/// the elicit call returns a timeout error which `normalize` treats as `Refused`
/// — a stuck prompt fails safe (no action) instead of holding the request open
/// indefinitely.
const ELICIT_TIMEOUT: Duration = Duration::from_secs(300);

/// Structured payload requested from the user for a high-impact operation. A single
/// boolean: the client renders a confirm prompt from the generated schema.
/// `Accept` with `confirm=true` proceeds; anything else aborts.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct OperationConfirmation {
    /// Set true to confirm and run this high-impact operation.
    pub confirm: bool,
}

rmcp::elicit_safe!(OperationConfirmation);

/// How a high-impact action should be handled on the MCP surface.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OperationGate {
    /// The user explicitly approved the elicitation prompt.
    Proceed,
    /// The user declined/cancelled (or the prompt failed) — do NOT run it.
    Declined,
}

/// The outcome of an elicitation round-trip, normalized away from rmcp's
/// `Result<Option<_>, ElicitationError>`. This intermediate exists so the gate
/// decision ([`classify`]) is a pure, fully unit-testable function: rmcp's
/// `ElicitationError` is `#[non_exhaustive]` and cannot be constructed by a
/// downstream crate, so the error arms of [`normalize`] are not directly
/// testable — but everything downstream of this enum is.
#[derive(Debug, PartialEq, Eq)]
enum ElicitOutcome {
    /// User explicitly approved (`confirm = true`).
    Confirmed,
    /// Fail-safe bucket: user declined/cancelled, sent `confirm=false`/no content,
    /// or the prompt failed for any reason (parse/transport/timeout).
    Refused,
    /// The client does not support elicitation at all.
    Unsupported,
}

/// The elicitation prompt shown to the user before a high-impact operation.
pub(crate) fn confirm_message(
    action: &str,
    service: &str,
    effect: crate::actions::OperationEffect,
) -> String {
    let (label, impact) = match effect {
        crate::actions::OperationEffect::Destructive => (
            "destructive",
            "This can permanently delete or replace data and may not be reversible.",
        ),
        crate::actions::OperationEffect::Disruptive => (
            "disruptive",
            "This can interrupt active work, playback, or the running application.",
        ),
        crate::actions::OperationEffect::ReadOnly | crate::actions::OperationEffect::Mutating => {
            ("mutating", "This operation changes service state.")
        }
    };
    format!(
        "Confirm {label} action '{action}' on service '{service}'. {impact} Approve to proceed."
    )
}

/// Pure decision: map a normalized [`ElicitOutcome`] to an [`OperationGate`]. Fully
/// unit-testable (no `Peer`, no rmcp error types).
fn classify(outcome: ElicitOutcome) -> OperationGate {
    match outcome {
        ElicitOutcome::Confirmed => OperationGate::Proceed,
        ElicitOutcome::Refused | ElicitOutcome::Unsupported => OperationGate::Declined,
    }
}

/// Normalize an rmcp elicit result to an [`ElicitOutcome`]. The non-`Confirmed`
/// collapse is the fail-safe default: anything other than an explicit
/// `confirm=true` (decline, cancel, empty, parse/transport error, timeout)
/// becomes `Refused`; only a declared missing capability is `Unsupported`. The
/// `Ok` arms are unit-tested; the `Err` arms cannot be (non-constructible
/// `#[non_exhaustive]` error), so they are kept to a trivial, obviously-safe
/// match.
fn normalize(result: Result<Option<OperationConfirmation>, ElicitationError>) -> ElicitOutcome {
    match result {
        Ok(Some(OperationConfirmation { confirm: true })) => ElicitOutcome::Confirmed,
        Ok(Some(OperationConfirmation { confirm: false })) | Ok(None) => ElicitOutcome::Refused,
        Err(ElicitationError::CapabilityNotSupported) => ElicitOutcome::Unsupported,
        Err(_) => ElicitOutcome::Refused,
    }
}

/// Gate a high-impact `action` targeting `service` on the MCP surface.
///
/// 1. Client can't elicit → [`OperationGate::Declined`] (fail closed).
/// 2. Otherwise prompt (with a timeout) and map the outcome ([`normalize`] +
///    [`classify`]) — there is no way to skip this prompt from the call
///    arguments.
pub(crate) async fn gate_operation(
    peer: &Peer<RoleServer>,
    action: &str,
    service: &str,
    effect: crate::actions::OperationEffect,
) -> OperationGate {
    if peer.supported_elicitation_modes().is_empty() {
        return OperationGate::Declined;
    }
    let result = peer
        .elicit_with_timeout::<OperationConfirmation>(
            confirm_message(action, service, effect),
            Some(ELICIT_TIMEOUT),
        )
        .await;
    classify(normalize(result))
}

#[cfg(test)]
#[path = "elicit_tests.rs"]
mod tests;
