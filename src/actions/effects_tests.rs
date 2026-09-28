use serde_json::json;

use super::{OperationEffect, classify_operation};
use crate::{config::ServiceKind, openapi};

fn effect(kind: ServiceKind, operation: &str, body: Option<&serde_json::Value>) -> OperationEffect {
    let spec = openapi::find_operation(kind, operation)
        .unwrap_or_else(|| panic!("missing {} operation {operation}", kind.as_str()));
    classify_operation(kind, spec, body)
}

#[test]
fn qbittorrent_operation_metadata_distinguishes_high_impact_calls() {
    for (operation, expected) in [
        ("post_torrents_delete", OperationEffect::Destructive),
        ("post_rss_remove_item", OperationEffect::Destructive),
        ("post_search_install_plugin", OperationEffect::Disruptive),
        ("post_app_shutdown", OperationEffect::Disruptive),
        ("post_torrents_stop", OperationEffect::Mutating),
    ] {
        assert_eq!(effect(ServiceKind::Qbittorrent, operation, None), expected);
    }

    let stop = json!({"shareLimitAction": "Stop"});
    let remove = json!({"shareLimitAction": "Remove"});
    assert_eq!(
        effect(ServiceKind::Qbittorrent, "post_torrents_add", Some(&stop)),
        OperationEffect::Mutating
    );
    assert_eq!(
        effect(ServiceKind::Qbittorrent, "post_torrents_add", Some(&remove)),
        OperationEffect::Destructive
    );
}

#[test]
fn plex_and_servarr_high_impact_metadata_is_preserved() {
    for (kind, operation, expected) in [
        (
            ServiceKind::Plex,
            "empty_trash",
            OperationEffect::Destructive,
        ),
        (
            ServiceKind::Plex,
            "terminate_session",
            OperationEffect::Disruptive,
        ),
        (
            ServiceKind::Sonarr,
            "post_system_backup_restore_by_id",
            OperationEffect::Destructive,
        ),
        (
            ServiceKind::Radarr,
            "post_system_restart",
            OperationEffect::Disruptive,
        ),
    ] {
        assert_eq!(effect(kind, operation, None), expected);
    }

    let restart = json!({"name": "Restart"});
    assert_eq!(
        effect(ServiceKind::Sonarr, "post_command", Some(&restart)),
        OperationEffect::Disruptive
    );
}

#[test]
fn high_impact_flag_excludes_reads_and_ordinary_mutations() {
    assert!(!OperationEffect::ReadOnly.is_high_impact());
    assert!(!OperationEffect::Mutating.is_high_impact());
    assert!(OperationEffect::Disruptive.is_high_impact());
    assert!(OperationEffect::Destructive.is_high_impact());
}
