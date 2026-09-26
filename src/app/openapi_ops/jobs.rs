//! Opt-in bounded outcome tracking for Servarr commands.

use std::time::Duration;

use anyhow::{Result, anyhow, bail, ensure};
use serde_json::{Map, Value, json};
use tokio::time::{Instant, sleep_until, timeout_at};

use crate::app::YarrService;
use crate::config::{ServiceConfig, ServiceKind};
use crate::openapi::{self, OperationSpec};

const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const MAX_TIMEOUT_SECONDS: u64 = 20;
const DEFAULT_POLL_INTERVAL_MS: u64 = 500;
const MIN_POLL_INTERVAL_MS: u64 = 100;
const MAX_POLL_INTERVAL_MS: u64 = 5_000;

pub(super) struct WaitOptions {
    timeout: Duration,
    poll_interval: Duration,
}

pub(super) fn wait_options(
    kind: ServiceKind,
    operation: &OperationSpec,
    args: &Value,
) -> Result<Option<WaitOptions>> {
    let args = args
        .as_object()
        .ok_or_else(|| anyhow!("operation `{}` args must be an object", operation.name))?;
    let Some(value) = args.get("waitForCompletion") else {
        return Ok(None);
    };
    ensure!(
        matches!(kind, ServiceKind::Sonarr | ServiceKind::Radarr)
            && operation.name == "post_command",
        "waitForCompletion is only supported for Sonarr and Radarr post_command"
    );
    let controls = value
        .as_object()
        .ok_or_else(|| anyhow!("waitForCompletion must be an object"))?;
    for field in controls.keys() {
        ensure!(
            matches!(field.as_str(), "timeoutSeconds" | "pollIntervalMs"),
            "unknown waitForCompletion field `{field}`"
        );
    }
    let timeout_seconds = integer_control(
        controls,
        "timeoutSeconds",
        DEFAULT_TIMEOUT_SECONDS,
        1,
        MAX_TIMEOUT_SECONDS,
    )?;
    let poll_interval_ms = integer_control(
        controls,
        "pollIntervalMs",
        DEFAULT_POLL_INTERVAL_MS,
        MIN_POLL_INTERVAL_MS,
        MAX_POLL_INTERVAL_MS,
    )?;
    Ok(Some(WaitOptions {
        timeout: Duration::from_secs(timeout_seconds),
        poll_interval: Duration::from_millis(poll_interval_ms),
    }))
}

fn integer_control(
    controls: &Map<String, Value>,
    field: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64> {
    let Some(value) = controls.get(field) else {
        return Ok(default);
    };
    let Some(value) = value.as_u64() else {
        bail!("waitForCompletion.{field} must be an integer from {min} through {max}");
    };
    ensure!(
        (min..=max).contains(&value),
        "waitForCompletion.{field} must be an integer from {min} through {max}"
    );
    Ok(value)
}

pub(super) async fn submit_and_wait(
    service: &YarrService,
    config: &ServiceConfig,
    operation: &OperationSpec,
    args: &Value,
    options: WaitOptions,
) -> Result<Value> {
    let deadline = Instant::now() + options.timeout;
    let poll_operation = openapi::find_operation(config.kind, "get_command_by_id")
        .ok_or_else(|| anyhow!("generated get_command_by_id operation is unavailable"))?;
    let submitted = timeout_at(
        deadline,
        service.execute_operation_once(config, operation, args),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "post_command timed out before a command id was returned; submission outcome is unknown"
        )
    })??;
    let command_id = command_id(&submitted)?;
    if let Some(status) = terminal_status(&submitted) {
        return Ok(outcome(command_id, status, true, submitted, None));
    }

    let poll_args = json!({ "id": command_id });
    let mut latest = submitted;

    loop {
        let wake = (Instant::now() + options.poll_interval).min(deadline);
        sleep_until(wake).await;
        if Instant::now() >= deadline {
            return Ok(outcome(
                command_id,
                "timed_out",
                false,
                latest,
                Some("deadline reached; call get_command_by_id with commandId to resume"),
            ));
        }

        match timeout_at(
            deadline,
            service.execute_operation_once(config, poll_operation, &poll_args),
        )
        .await
        {
            Err(_) => {
                return Ok(outcome(
                    command_id,
                    "timed_out",
                    false,
                    latest,
                    Some("deadline reached; call get_command_by_id with commandId to resume"),
                ));
            }
            Ok(Err(_)) => {
                return Ok(outcome(
                    command_id,
                    "unknown",
                    false,
                    latest,
                    Some("poll failed; call get_command_by_id with commandId to resume"),
                ));
            }
            Ok(Ok(command)) => {
                latest = command;
                match poll_status(&latest, command_id) {
                    Ok(Some(status)) => {
                        return Ok(outcome(command_id, status, true, latest, None));
                    }
                    Ok(None) => {}
                    Err(_) => {
                        return Ok(outcome(
                            command_id,
                            "unknown",
                            false,
                            latest,
                            Some(
                                "poll response was invalid; call get_command_by_id with commandId to resume",
                            ),
                        ));
                    }
                }
            }
        }
    }
}

fn command_id(command: &Value) -> Result<i64> {
    command
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            anyhow!(
                "post_command response did not include a positive integer command id; command outcome cannot be tracked"
            )
        })
}

fn terminal_status(command: &Value) -> Option<&'static str> {
    terminal_status_value(command.get("status")?.as_str()?)
}

fn poll_status(command: &Value, command_id: i64) -> Result<Option<&'static str>> {
    ensure!(
        command.get("id").and_then(Value::as_i64) == Some(command_id),
        "poll response command id did not match submitted command"
    );
    let status = command
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("poll response did not include a string status"))?;
    if matches!(status.to_ascii_lowercase().as_str(), "queued" | "started") {
        return Ok(None);
    }
    terminal_status_value(status)
        .map(Some)
        .ok_or_else(|| anyhow!("poll response included an unknown status"))
}

fn terminal_status_value(status: &str) -> Option<&'static str> {
    match status.to_ascii_lowercase().as_str() {
        "completed" => Some("completed"),
        "failed" | "orphaned" => Some("failed"),
        "aborted" => Some("aborted"),
        "cancelled" => Some("cancelled"),
        _ => None,
    }
}

fn outcome(
    command_id: i64,
    status: &'static str,
    finished: bool,
    command: Value,
    note: Option<&'static str>,
) -> Value {
    let mut value = json!({
        "commandId": command_id,
        "status": status,
        "finished": finished,
        "command": command,
    });
    if let Some(note) = note {
        value["note"] = Value::String(note.to_owned());
    }
    value
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
