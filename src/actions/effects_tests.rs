use serde_json::json;

use super::*;
use crate::{
    app::YarrService,
    config::{ServiceConfig, ServiceKind, YarrConfig},
    yarr::YarrClient,
};

fn service(kinds: &[(&str, ServiceKind)]) -> YarrService {
    let config = YarrConfig {
        services: kinds
            .iter()
            .map(|(name, kind)| ServiceConfig {
                name: (*name).to_owned(),
                kind: *kind,
                base_url: "http://localhost:1".to_owned(),
                ..ServiceConfig::default()
            })
            .collect(),
    };
    YarrService::new(YarrClient::new(&config).unwrap(), config)
}

fn generated(service: &str, op: &str, args: Value) -> YarrAction {
    YarrAction::Op {
        service: service.to_owned(),
        op: op.to_owned(),
        args,
    }
}

#[test]
fn qbittorrent_post_effects_apply_to_generated_and_raw_calls() {
    let service = service(&[("qbit_movies", ServiceKind::Qbittorrent)]);
    for (suffix, expected) in [
        ("torrents/delete", OperationEffect::Destructive),
        ("rss/removeItem", OperationEffect::Destructive),
        ("rss/removeRule", OperationEffect::Destructive),
        ("search/uninstallPlugin", OperationEffect::Destructive),
        ("search/installPlugin", OperationEffect::Disruptive),
        ("search/updatePlugins", OperationEffect::Disruptive),
        ("app/shutdown", OperationEffect::Disruptive),
        ("app/deleteAPIKey", OperationEffect::Disruptive),
        ("app/rotateAPIKey", OperationEffect::Disruptive),
        ("app/setPreferences", OperationEffect::Disruptive),
        ("torrents/stop", OperationEffect::Mutating),
        ("torrents/start", OperationEffect::Mutating),
        ("torrents/recheck", OperationEffect::Mutating),
        ("transfer/setDownloadLimit", OperationEffect::Mutating),
    ] {
        let path = format!("/api/v2/{suffix}");
        let spec = openapi::operations_for_kind(ServiceKind::Qbittorrent)
            .iter()
            .find(|op| op.path == path && op.method == HttpMethod::Post)
            .unwrap_or_else(|| panic!("missing qBittorrent route {path}"));
        for action in [
            generated("qbit_movies", spec.name, json!({"body": {}})),
            YarrAction::ApiPost {
                service: "qbit_movies".into(),
                path: path.clone(),
                body: json!({}),
            },
        ] {
            assert_eq!(
                operation_effect(&service, &action).unwrap(),
                expected,
                "{path}"
            );
        }
    }
}

#[test]
fn qbittorrent_automatic_removal_is_destructive_even_when_scheduled_for_later() {
    let service = service(&[("qbit_tv", ServiceKind::Qbittorrent)]);
    for path in ["/api/v2/torrents/add", "/api/v2/torrents/setShareLimits"] {
        let spec = openapi::operations_for_kind(ServiceKind::Qbittorrent)
            .iter()
            .find(|op| op.path == path)
            .unwrap();
        for (setting, expected) in [
            ("Stop", OperationEffect::Mutating),
            ("Remove", OperationEffect::Destructive),
            ("RemoveWithContent", OperationEffect::Destructive),
        ] {
            let body = json!({"shareLimitAction": setting});
            for action in [
                generated("qbit_tv", spec.name, json!({"body": body})),
                YarrAction::ApiPost {
                    service: "qbit_tv".into(),
                    path: path.into(),
                    body: body.clone(),
                },
            ] {
                assert_eq!(
                    operation_effect(&service, &action).unwrap(),
                    expected,
                    "{path} {setting}"
                );
            }
        }
    }
}

#[test]
fn plex_non_delete_high_impact_operations_require_confirmation() {
    let service = service(&[("plex", ServiceKind::Plex)]);
    for (op, expected) in [
        ("empty_trash", OperationEffect::Destructive),
        ("terminate_session", OperationEffect::Disruptive),
        ("apply_updates", OperationEffect::Disruptive),
    ] {
        assert_eq!(
            operation_effect(&service, &generated("plex", op, json!({}))).unwrap(),
            expected,
            "{op}"
        );
    }
}

#[test]
fn arr_restore_process_control_and_dangerous_commands_require_confirmation() {
    let service = service(&[
        ("sonarr", ServiceKind::Sonarr),
        ("radarr", ServiceKind::Radarr),
    ]);
    for name in ["sonarr", "radarr"] {
        for op in [
            "post_system_backup_restore_by_id",
            "post_system_backup_restore_upload",
        ] {
            assert_eq!(
                operation_effect(&service, &generated(name, op, json!({}))).unwrap(),
                OperationEffect::Destructive,
                "{name}.{op}"
            );
        }
        for op in ["post_system_restart", "post_system_shutdown"] {
            assert_eq!(
                operation_effect(&service, &generated(name, op, json!({}))).unwrap(),
                OperationEffect::Disruptive,
                "{name}.{op}"
            );
        }
        for command in ["ApplicationUpdate", "ResetApiKey", "restart", "SHUTDOWN"] {
            assert_eq!(
                operation_effect(
                    &service,
                    &generated(name, "post_command", json!({"body": {"name": command}})),
                )
                .unwrap(),
                OperationEffect::Disruptive,
                "{name} command {command}"
            );
        }
    }
}

#[test]
fn raw_and_generated_calls_share_route_semantics() {
    let service = service(&[("plex", ServiceKind::Plex), ("sonarr", ServiceKind::Sonarr)]);
    let cases = [
        (
            YarrAction::ApiPut {
                service: "plex".into(),
                path: "/library/sections/7/emptyTrash?ignored=true".into(),
                body: json!({}),
            },
            OperationEffect::Destructive,
        ),
        (
            YarrAction::ApiPost {
                service: "plex".into(),
                path: "/status/sessions/terminate?sessionId=abc".into(),
                body: json!({}),
            },
            OperationEffect::Disruptive,
        ),
        (
            YarrAction::ApiPost {
                service: "sonarr".into(),
                path: "/api/v3/system/backup/restore/4".into(),
                body: json!({}),
            },
            OperationEffect::Destructive,
        ),
        (
            YarrAction::ApiPost {
                service: "sonarr".into(),
                path: "/api/v3/command".into(),
                body: json!({"commandName": "ApplicationUpdate"}),
            },
            OperationEffect::Disruptive,
        ),
    ];
    for (action, expected) in cases {
        assert_eq!(operation_effect(&service, &action).unwrap(), expected);
    }
}

#[test]
fn ordinary_known_mutations_are_not_unnecessarily_gated() {
    let service = service(&[("plex", ServiceKind::Plex), ("sonarr", ServiceKind::Sonarr)]);
    assert_eq!(
        operation_effect(&service, &generated("plex", "set_preferences", json!({})),).unwrap(),
        OperationEffect::Mutating
    );
    assert_eq!(
        operation_effect(
            &service,
            &generated(
                "sonarr",
                "post_command",
                json!({"body": {"name": "RssSync"}}),
            ),
        )
        .unwrap(),
        OperationEffect::Mutating
    );
    assert_eq!(
        operation_effect(
            &service,
            &YarrAction::ApiPost {
                service: "sonarr".into(),
                path: "/api/v3/tag".into(),
                body: json!({"label": "ordinary"}),
            },
        )
        .unwrap(),
        OperationEffect::Mutating
    );
}

#[test]
fn existing_curated_destructive_metadata_still_requires_confirmation() {
    let service = service(&[("tautulli", ServiceKind::Tautulli)]);
    assert_eq!(
        operation_effect(
            &service,
            &YarrAction::Curated {
                name: "stats_delete_image_cache",
                params: json!({"service": "tautulli"}),
            },
        )
        .unwrap(),
        OperationEffect::Destructive
    );
    assert_eq!(
        operation_effect(&service, &YarrAction::SnippetDelete { name: "x".into() }).unwrap(),
        OperationEffect::Mutating
    );
}

#[test]
fn unknown_passthrough_writes_are_conservative_and_invalid_calls_do_not_fall_through() {
    let service = service(&[("sonarr", ServiceKind::Sonarr)]);
    assert_eq!(
        operation_effect(
            &service,
            &YarrAction::ApiPost {
                service: "sonarr".into(),
                path: "/api/v3/new-unreviewed-write".into(),
                body: json!({}),
            },
        )
        .unwrap(),
        OperationEffect::Disruptive
    );
    assert!(
        operation_effect(
            &service,
            &YarrAction::ApiPut {
                service: "sonarr".into(),
                path: "/api/v3/../escape".into(),
                body: json!({}),
            },
        )
        .is_err()
    );
    assert!(
        operation_effect(
            &service,
            &generated("sonarr", "no_such_operation", json!({})),
        )
        .is_err()
    );
}

#[test]
fn template_match_requires_exact_literals_and_nonempty_placeholders() {
    assert!(path_matches_template(
        "/library/sections/{sectionId}/emptyTrash",
        "/library/sections/9/emptyTrash"
    ));
    assert!(!path_matches_template(
        "/library/sections/{sectionId}/emptyTrash",
        "/library/sections//emptyTrash"
    ));
    assert!(!path_matches_template(
        "/library/sections/{sectionId}/emptyTrash",
        "/library/sections/9/refresh"
    ));
}
