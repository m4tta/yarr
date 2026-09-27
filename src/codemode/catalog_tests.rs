//! Discovery catalog tests.

use super::*;
use crate::config::ServiceKind;
use serde_json::json;

fn services() -> Vec<(String, ServiceKind)> {
    vec![
        ("sonarr".to_string(), ServiceKind::Sonarr),
        ("radarr".to_string(), ServiceKind::Radarr),
        ("plex".to_string(), ServiceKind::Plex),
    ]
}

#[test]
fn catalog_paths_are_fully_qualified_per_service() {
    let cat = build_catalog(&services());
    let paths: Vec<&str> = cat.iter().map(CatalogEntry::path).collect();
    // Spec-backed services expose their generated operations (+ service_status),
    // each prefixed with the service name. The service is baked into the path.
    assert!(paths.contains(&"sonarr.service_status"));
    assert!(paths.contains(&"radarr.service_status"));
    assert!(paths.contains(&"sonarr.get_series"));
    assert!(paths.contains(&"radarr.get_movie"));
    assert!(paths.contains(&"sonarr.delete_series_by_id"));
    // No bare action names leak in — discovery only offers callable paths.
    assert!(!paths.contains(&"get_series"));
    assert!(!paths.contains(&"integrations"));
    assert!(!paths.contains(&"codemode"));
}

#[test]
fn each_entry_carries_its_service() {
    let cat = build_catalog(&services());
    let series = cat
        .iter()
        .find(|e| e.path() == "sonarr.get_series")
        .unwrap();
    assert_eq!(series.service(), Some("sonarr"));
    assert_eq!(series.method(), "get_series");
    assert_eq!(series.kind(), "operation");
    // Capability carries the OpenAPI tag, not "infra".
    assert_ne!(series.capability_label(), "infra");
    assert!(!series.description().is_empty());
    // `service` is baked in, never a param the script passes.
    assert!(!series.required_params().contains(&"service"));
    // A destructive operation is marked so the MCP layer can require confirmation.
    let del = cat
        .iter()
        .find(|e| e.path() == "sonarr.delete_series_by_id")
        .unwrap();
    assert!(del.destructive());
}

#[test]
fn waitable_command_controls_are_discoverable() {
    let cat = build_catalog(&services());
    for path in ["sonarr.post_command", "radarr.post_command"] {
        let command = cat.iter().find(|entry| entry.path() == path).unwrap();
        let description = command.description();
        assert!(description.contains("waitForCompletion"));
        assert!(description.contains("timeoutSeconds"));
        assert!(description.contains("pollIntervalMs"));
        assert!(description.contains("commandId"));
    }
}

#[test]
fn catalog_paths_are_unique() {
    let cat = build_catalog(&services());
    let mut paths: Vec<&str> = cat.iter().map(CatalogEntry::path).collect();
    paths.sort_unstable();
    let mut deduped = paths.clone();
    deduped.dedup();
    assert_eq!(paths, deduped, "catalog callable paths must be unique");
}

#[test]
fn raw_api_client_is_documented_service_agnostically() {
    let cat = build_catalog(&services());
    let get = cat
        .iter()
        .find(|e| e.path() == "api.<service>.get")
        .unwrap();
    assert_eq!(get.scope().as_str(), "write"); // api_get requires write scope
    assert!(!get.destructive());
    assert!(get.service().is_none(), "raw-api docs are service-agnostic");

    let del = cat
        .iter()
        .find(|e| e.path() == "api.<service>.delete")
        .unwrap();
    assert!(del.destructive(), "api_delete is destructive");
}

#[test]
fn catalog_json_is_valid_json_array() {
    let json = catalog_json(&services());
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_array());
    assert!(
        parsed
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.get("kind").and_then(|k| k.as_str()) == Some("operation"))
    );
    assert_eq!(
        parsed.as_array().unwrap().len(),
        build_catalog(&services()).len()
    );
}

#[test]
fn empty_services_yields_only_raw_api_docs() {
    let cat = build_catalog(&[]);
    // No services configured → only the four service-agnostic raw-API entries.
    assert_eq!(cat.len(), 4);
    assert!(cat.iter().all(|e| e.service().is_none()));
}

#[test]
fn qbittorrent_catalog_keeps_curated_helpers_out_of_sabnzbd() {
    let cat = build_catalog(&[
        ("qbit_movies".to_string(), ServiceKind::Qbittorrent),
        ("sabnzbd".to_string(), ServiceKind::Sabnzbd),
    ]);
    let paths = cat.iter().map(CatalogEntry::path).collect::<Vec<_>>();
    assert!(paths.contains(&"qbit_movies.download_set_limits"));
    assert!(paths.contains(&"qbit_movies.download_tags"));
    assert!(!paths.contains(&"sabnzbd.download_set_limits"));
    assert!(!paths.contains(&"sabnzbd.download_tags"));
}

#[test]
fn operation_required_params_use_parameter_and_body_requiredness() {
    let spec = crate::openapi::operations_for_kind(ServiceKind::Sonarr)
        .iter()
        .find(|operation| operation.name == "delete_queue_bulk")
        .unwrap();
    let entry = operation_entry("sonarr", ServiceKind::Sonarr, spec);
    let json = serde_json::to_value(&entry).unwrap();

    assert!(
        json["required_params"]
            .as_array()
            .is_some_and(|params| !params.iter().any(|param| param == "body"))
    );

    let required_query = crate::openapi::operations_for_kind(ServiceKind::Qbittorrent)
        .iter()
        .find_map(|operation| {
            operation
                .parameters
                .iter()
                .find(|parameter| {
                    parameter.required
                        && parameter.location == crate::openapi::ParameterLocation::Query
                })
                .map(|parameter| (operation, parameter))
        })
        .expect("qBittorrent spec should contain a required query parameter");
    let json = serde_json::to_value(operation_entry(
        "qbit",
        ServiceKind::Qbittorrent,
        required_query.0,
    ))
    .unwrap();
    assert!(
        json["required_params"]
            .as_array()
            .unwrap()
            .iter()
            .any(|param| param == required_query.1.name)
    );
}

#[test]
fn qbittorrent_describe_includes_wire_metadata() {
    let operations = crate::openapi::operations_for_kind(ServiceKind::Qbittorrent);
    assert!(
        !operations.is_empty(),
        "qBittorrent OpenAPI registry is empty"
    );
    let operation = operations
        .iter()
        .find(|operation| !operation.parameters.is_empty() || operation.request_body.is_some())
        .expect("qBittorrent operation with inputs");
    let json =
        serde_json::to_value(operation_entry("qbit", ServiceKind::Qbittorrent, operation)).unwrap();
    if !operation.parameters.is_empty() {
        let parameters = json["parameters"].as_array().unwrap();
        assert_eq!(parameters.len(), operation.parameters.len());
        assert!(parameters.iter().all(|parameter| {
            parameter["name"].is_string()
                && parameter["location"].is_string()
                && parameter["required"].is_boolean()
                && parameter["schema"].is_object()
        }));
    }
    if let Some(body) = operation.request_body {
        assert_eq!(json["request_body"]["required"], body.required);
        assert_eq!(
            json["request_body"]["representations"]
                .as_array()
                .unwrap()
                .len(),
            body.representations.len()
        );
    }
}

#[test]
fn qbittorrent_multipart_describe_uses_real_top_level_file_controls() {
    let operations = crate::openapi::operations_for_kind(ServiceKind::Qbittorrent);
    let describe = |path| {
        let operation = operations
            .iter()
            .find(|operation| operation.path == path)
            .unwrap();
        serde_json::to_value(operation_entry("qbit", ServiceKind::Qbittorrent, operation)).unwrap()
    };

    let add = describe("/api/v2/torrents/add");
    assert!(
        !add["required_params"]
            .as_array()
            .unwrap()
            .contains(&json!("body"))
    );
    let body = &add["request_body"];
    assert_eq!(body["required"], true);
    assert_eq!(body["bodyArgumentRequired"], false);
    assert_eq!(
        body["multipart"]["requiredAlternatives"],
        json!([["body.urls"], ["multipartFileBase64"]])
    );
    assert_eq!(
        body["multipart"]["fileFields"],
        json!([{"name":"torrents","mediaType":"application/x-bittorrent"}])
    );
    let controls = body["multipart"]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|control| control["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        controls,
        [
            "multipartFileBase64",
            "fileName",
            "multipartField",
            "contentType"
        ]
    );
    let add_schema = &body["representations"][0]["schema"];
    assert!(add_schema["properties"].get("torrents").is_none());
    assert!(add_schema["properties"].get("urls").is_some());
    assert_eq!(add_schema["anyOf"], json!([{"required":["urls"]}]));

    let parsed = describe("/api/v2/torrents/parseMetadata");
    assert!(
        !parsed["required_params"]
            .as_array()
            .unwrap()
            .contains(&json!("body"))
    );
    assert_eq!(
        parsed["request_body"]["multipart"]["requiredAlternatives"],
        json!([["multipartFileBase64"]])
    );
    assert!(
        parsed["request_body"]["representations"][0]["schema"]["properties"]
            .get("torrent")
            .is_none()
    );
}

#[test]
fn disruptive_post_is_marked_for_confirmation_in_catalog() {
    let shutdown = crate::openapi::operations_for_kind(ServiceKind::Qbittorrent)
        .iter()
        .find(|operation| operation.path == "/api/v2/app/shutdown")
        .expect("qBittorrent shutdown operation");
    assert!(operation_entry("qbit", ServiceKind::Qbittorrent, shutdown).destructive());
}
