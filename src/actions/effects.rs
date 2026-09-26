//! Semantic operation-effect policy shared by every MCP dispatch surface.
//!
//! HTTP verbs are an incomplete risk signal: Plex empties library trash with a
//! PUT, terminates playback with a POST, and applies updates with a PUT. Servarr
//! similarly restores backups and controls the process with POSTs. This module
//! classifies the resolved upstream operation once so flat MCP and inner Code
//! Mode calls cannot drift into separate policy implementations.

use anyhow::{Result, anyhow};
use serde_json::Value;

use super::{YarrAction, curated_command};
use crate::{
    app::YarrService,
    config::ServiceKind,
    openapi::{self, HttpMethod, OperationSpec},
    yarr::validate_safe_path,
};

/// Operational impact of one fully parsed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationEffect {
    ReadOnly,
    Mutating,
    /// Interrupts active work or changes the running application and therefore
    /// requires an interactive confirmation on MCP.
    Disruptive,
    /// Can permanently delete or replace operator data and therefore requires
    /// an interactive confirmation on MCP.
    Destructive,
}

impl OperationEffect {
    pub const fn requires_confirmation(self) -> bool {
        matches!(self, Self::Disruptive | Self::Destructive)
    }
}

/// Classify one parsed action. This is policy only: the CLI deliberately does
/// not call it as a gate and retains its trusted-local-operator behavior.
pub fn operation_effect(service: &YarrService, action: &YarrAction) -> Result<OperationEffect> {
    match action {
        YarrAction::ServiceStatus { .. } | YarrAction::ApiGet { .. } | YarrAction::Help => {
            Ok(OperationEffect::ReadOnly)
        }
        YarrAction::ApiPost {
            service: name,
            path,
            body,
        } => classify_passthrough(service, name, HttpMethod::Post, path, Some(body)),
        YarrAction::ApiPut {
            service: name,
            path,
            body,
        } => classify_passthrough(service, name, HttpMethod::Put, path, Some(body)),
        YarrAction::ApiDelete {
            service: name,
            path,
            body,
        } => classify_passthrough(service, name, HttpMethod::Delete, path, body.as_ref()),
        YarrAction::Op {
            service: name,
            op,
            args,
        } => {
            let kind = configured_kind(service, name)?;
            let spec = openapi::find_operation(kind, op).ok_or_else(|| {
                anyhow!("unknown or unsupported {} operation `{op}`", kind.as_str())
            })?;
            let body = args.as_object().and_then(|args| args.get("body"));
            Ok(classify_operation(kind, spec, body))
        }
        YarrAction::Curated { name, .. } => {
            let descriptor = curated_command(name)
                .ok_or_else(|| anyhow!("curated command `{name}` is not registered"))?;
            Ok(effect_from_flags(
                descriptor.mutates,
                descriptor.destructive,
            ))
        }
        YarrAction::CodeMode { .. }
        | YarrAction::SnippetSave { .. }
        | YarrAction::SnippetRun { .. }
        | YarrAction::SnippetDelete { .. } => Ok(OperationEffect::Mutating),
        YarrAction::SnippetList => Ok(OperationEffect::ReadOnly),
    }
}

fn classify_passthrough(
    service: &YarrService,
    service_name: &str,
    method: HttpMethod,
    path: &str,
    body: Option<&Value>,
) -> Result<OperationEffect> {
    // Match only paths that the transport itself can safely represent. Invalid
    // paths fail before policy or dispatch rather than falling through as a
    // supposedly ordinary mutation.
    validate_safe_path(path)?;
    let kind = configured_kind(service, service_name)?;
    let path = path_only(path);
    if let Some(spec) = openapi::operations_for_kind(kind)
        .iter()
        .find(|spec| spec.method == method && path_matches_template(spec.path, path))
    {
        return Ok(classify_operation(kind, spec, body));
    }

    // An arbitrary passthrough write has no spec-derived semantics to prove it
    // ordinary. Preserve reads and DELETE behavior, but fail closed with an MCP
    // confirmation for unknown POST/PUT routes.
    Ok(match method {
        HttpMethod::Get => OperationEffect::ReadOnly,
        HttpMethod::Delete => OperationEffect::Destructive,
        HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch => OperationEffect::Disruptive,
    })
}

fn configured_kind(service: &YarrService, name: &str) -> Result<ServiceKind> {
    service
        .kind_of(name)?
        .ok_or_else(|| anyhow!("unknown yarr service: {name}"))
}

fn classify_operation(
    kind: ServiceKind,
    spec: &OperationSpec,
    body: Option<&Value>,
) -> OperationEffect {
    if kind == ServiceKind::Plex {
        match (spec.method, spec.path) {
            (HttpMethod::Put, "/library/sections/{sectionId}/emptyTrash") => {
                return OperationEffect::Destructive;
            }
            (HttpMethod::Post, "/status/sessions/terminate")
            | (HttpMethod::Put, "/updater/apply") => return OperationEffect::Disruptive,
            _ => {}
        }
    }

    if matches!(kind, ServiceKind::Sonarr | ServiceKind::Radarr) {
        match (spec.method, spec.path) {
            (HttpMethod::Post, "/api/v3/system/backup/restore/{id}")
            | (HttpMethod::Post, "/api/v3/system/backup/restore/upload") => {
                return OperationEffect::Destructive;
            }
            (HttpMethod::Post, "/api/v3/system/restart")
            | (HttpMethod::Post, "/api/v3/system/shutdown") => {
                return OperationEffect::Disruptive;
            }
            (HttpMethod::Post, "/api/v3/command") => {
                if let Some(effect) = servarr_command_effect(body) {
                    return effect;
                }
            }
            _ => {}
        }
    }

    match spec.method {
        HttpMethod::Get => OperationEffect::ReadOnly,
        HttpMethod::Delete => OperationEffect::Destructive,
        HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch => OperationEffect::Mutating,
    }
}

fn servarr_command_effect(body: Option<&Value>) -> Option<OperationEffect> {
    let command = body.and_then(|body| {
        body.get("name")
            .or_else(|| body.get("commandName"))
            .and_then(Value::as_str)
    })?;
    let normalized = command.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "backuprestore" | "restore" | "restorebackup"
    ) {
        return Some(OperationEffect::Destructive);
    }
    matches!(
        normalized.as_str(),
        "applicationupdate" | "resetapikey" | "restart" | "shutdown"
    )
    .then_some(OperationEffect::Disruptive)
}

fn effect_from_flags(mutates: bool, destructive: bool) -> OperationEffect {
    if destructive {
        OperationEffect::Destructive
    } else if mutates {
        OperationEffect::Mutating
    } else {
        OperationEffect::ReadOnly
    }
}

fn path_only(path: &str) -> &str {
    path.split_once('?')
        .map_or(path, |(path, _)| path)
        .trim_end_matches('/')
}

/// Match the path-template subset used by audited policy routes. Whole-segment
/// `{name}` placeholders match one non-empty concrete segment; literal segments
/// must match exactly. Embedded placeholders remain literal here and therefore
/// fall back to conservative passthrough policy rather than accidentally match.
fn path_matches_template(template: &str, path: &str) -> bool {
    let template = template.trim_end_matches('/');
    let mut expected = template.split('/');
    let mut actual = path.split('/');
    loop {
        match (expected.next(), actual.next()) {
            (None, None) => return true,
            (Some(expected), Some(actual)) => {
                let placeholder =
                    expected.starts_with('{') && expected.ends_with('}') && expected.len() > 2;
                if (placeholder && actual.is_empty()) || (!placeholder && expected != actual) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
