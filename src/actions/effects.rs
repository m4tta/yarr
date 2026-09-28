//! Semantic operation-effect metadata for discovery catalogs.
//!
//! HTTP verbs are an incomplete risk signal: Plex empties library trash with a
//! PUT, terminates playback with a POST, and applies updates with a PUT. Servarr
//! similarly restores backups and controls the process with POSTs. These labels
//! describe impact; authentication and scope checks govern dispatch.

use serde_json::Value;

use crate::{
    config::ServiceKind,
    openapi::{HttpMethod, OperationSpec},
};

/// Operational impact of one fully parsed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationEffect {
    ReadOnly,
    Mutating,
    /// Interrupts active work or changes the running application.
    Disruptive,
    /// Can permanently delete or replace operator data.
    Destructive,
}

impl OperationEffect {
    pub const fn is_high_impact(self) -> bool {
        matches!(self, Self::Disruptive | Self::Destructive)
    }
}

pub(crate) fn classify_operation(
    kind: ServiceKind,
    spec: &OperationSpec,
    body: Option<&Value>,
) -> OperationEffect {
    if kind == ServiceKind::Qbittorrent && spec.method == HttpMethod::Post {
        if matches!(
            spec.path,
            "/api/v2/torrents/add" | "/api/v2/torrents/setShareLimits"
        ) && matches!(
            body.and_then(|body| body.get("shareLimitAction"))
                .and_then(Value::as_str),
            Some("Remove" | "RemoveWithContent")
        ) {
            return OperationEffect::Destructive;
        }
        match spec.path {
            "/api/v2/torrents/delete"
            | "/api/v2/rss/removeItem"
            | "/api/v2/rss/removeRule"
            | "/api/v2/search/uninstallPlugin" => return OperationEffect::Destructive,
            "/api/v2/app/shutdown"
            | "/api/v2/app/deleteAPIKey"
            | "/api/v2/app/rotateAPIKey"
            | "/api/v2/app/setPreferences"
            | "/api/v2/search/installPlugin"
            | "/api/v2/search/updatePlugins" => return OperationEffect::Disruptive,
            _ => {}
        }
    }
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

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
