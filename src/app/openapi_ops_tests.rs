//! Tests for the generated-operation executor helpers.

use crate::openapi::{ParameterLocation, ParameterSpec, ParameterStyle};
use crate::testing::loopback_state;
use serde_json::json;

#[test]
fn http_method_maps_to_reqwest_method() {
    use crate::openapi::HttpMethod;

    assert_eq!(HttpMethod::Get.as_reqwest(), reqwest::Method::GET);
    assert_eq!(HttpMethod::Post.as_reqwest(), reqwest::Method::POST);
    assert_eq!(HttpMethod::Put.as_reqwest(), reqwest::Method::PUT);
    assert_eq!(HttpMethod::Delete.as_reqwest(), reqwest::Method::DELETE);
    assert_eq!(HttpMethod::Patch.as_reqwest(), reqwest::Method::PATCH);
}

#[test]
fn query_arg_values_support_scalars_and_arrays() {
    assert_eq!(
        super::query_arg_values("ids", &json!([1, "2", true])).unwrap(),
        vec![
            ("ids".to_string(), "1".to_string()),
            ("ids".to_string(), "2".to_string()),
            ("ids".to_string(), "true".to_string())
        ]
    );
    assert_eq!(
        super::query_arg_values("limit", &json!(25)).unwrap(),
        vec![("limit".to_string(), "25".to_string())]
    );
}

#[test]
fn query_arg_values_reject_nested_values_and_null() {
    for bad in [json!([{"x": 1}]), json!([[1]]), json!(null)] {
        assert!(super::query_arg_values("bad", &bad).is_err(), "{bad}");
    }
    assert_eq!(
        super::query_arg_values("filter", &json!({"x": 1})).unwrap(),
        vec![("x".to_string(), "1".to_string())]
    );
}

#[test]
fn parameter_serialization_honors_style_and_explode() {
    let repeated = ParameterSpec {
        name: "id",
        location: ParameterLocation::Query,
        required: false,
        schema: r#"{"type":"array"}"#,
        style: ParameterStyle::Form,
        explode: true,
    };
    assert_eq!(
        super::serialize_parameter(&repeated, &json!([1, 2])).unwrap(),
        vec![
            ("id".to_string(), "1".to_string()),
            ("id".to_string(), "2".to_string())
        ]
    );

    let compact = ParameterSpec {
        explode: false,
        ..repeated
    };
    assert_eq!(
        super::serialize_parameter(&compact, &json!([1, 2])).unwrap(),
        vec![("id".to_string(), "1,2".to_string())]
    );

    let deep = ParameterSpec {
        name: "filter",
        location: ParameterLocation::Query,
        required: false,
        schema: r#"{"type":"object"}"#,
        style: ParameterStyle::DeepObject,
        explode: true,
    };
    assert_eq!(
        super::serialize_parameter(&deep, &json!({"state": "ready", "year": 2026})).unwrap(),
        vec![
            ("filter[state]".to_string(), "ready".to_string()),
            ("filter[year]".to_string(), "2026".to_string())
        ]
    );
}

#[tokio::test]
async fn execute_operation_requires_each_path_param_before_dispatch() {
    // loopback configures `sonarr`. get_series_by_id needs `id`; omitting it must
    // fail at param resolution with a clear message BEFORE any network call.
    let service = loopback_state().service;
    let err = service
        .execute_operation("sonarr", "get_series_by_id", &json!({}))
        .await
        .expect_err("missing path param must error before HTTP");
    let msg = err.to_string();
    assert!(
        msg.contains("path param") && msg.contains("id"),
        "got: {msg}"
    );
}

#[tokio::test]
async fn execute_operation_rejects_a_string_for_an_integer_parameter() {
    let service = loopback_state().service;
    let err = service
        .execute_operation("sonarr", "get_series_by_id", &json!({"id": "7"}))
        .await
        .expect_err("invalid path parameter must error before HTTP");
    let msg = err.to_string();
    assert!(msg.contains("path parameter `id`"), "got: {msg}");
    assert!(msg.contains("integer"), "got: {msg}");
}

#[tokio::test]
async fn execute_operation_does_not_silently_drop_null_parameters() {
    let service = loopback_state().service;
    let err = service
        .execute_operation("sonarr", "get_series", &json!({"tvdbId": null}))
        .await
        .expect_err("explicit null parameter must error before HTTP");
    let msg = err.to_string();
    assert!(msg.contains("query parameter `tvdbId`"), "got: {msg}");
    assert!(msg.contains("cannot be null"), "got: {msg}");
}

#[tokio::test]
async fn execute_operation_rejects_unknown_op() {
    let service = loopback_state().service;
    let err = service
        .execute_operation("sonarr", "no_such_op", &json!({}))
        .await
        .expect_err("unknown op must error");
    assert!(err.to_string().contains("unknown"), "got: {err}");
}

#[path = "openapi_ops_tests/recording.rs"]
mod recording;

#[tokio::test]
async fn qbittorrent_generated_calls_validate_selectors_and_form_types_before_login() {
    use crate::config::{ServiceConfig, ServiceKind, YarrConfig};
    let config = YarrConfig {
        services: vec![ServiceConfig {
            name: "qbit_movies".into(),
            kind: ServiceKind::Qbittorrent,
            base_url: "http://127.0.0.1:1".into(),
            ..ServiceConfig::default()
        }],
    };
    let service =
        crate::app::YarrService::new(crate::yarr::YarrClient::new(&config).unwrap(), config);
    // No credentials or listening server: a validation error demonstrates that
    // neither authentication nor the operation's HTTP request was attempted.
    for (operation, args, expected) in [
        (
            "get_torrent_files",
            json!({}),
            "requires query parameter `hash`",
        ),
        (
            "post_torrents_delete",
            json!({"body":{"hashes":"abc"}}),
            "schema validation",
        ),
        (
            "post_torrents_delete",
            json!({"body":{"hashes":"abc","deleteFiles":"false"}}),
            "schema validation",
        ),
        (
            "post_transfer_set_download_limit",
            json!({"body":{"limit":"123"}}),
            "schema validation",
        ),
        (
            "post_app_set_preferences",
            json!({"body":{"json":{"dht":false}}}),
            "schema validation",
        ),
    ] {
        let error = service
            .execute_operation("qbit_movies", operation, &args)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{operation}: {error}");
    }
}

#[tokio::test]
async fn production_runtime_rejects_repo_relative_multipart_fixture() {
    let service = loopback_state().service;
    let error = service
        .execute_operation(
            "sonarr",
            "post_system_backup_restore_upload",
            &json!({"multipartFixture":"fixture.zip"}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("multipartFixture"), "{error:#}");
}
