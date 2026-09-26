use super::*;
use crate::{config::ServiceKind, openapi::find_operation, yarr::EncodedRequestBody};
use serde_json::json;

#[test]
fn binary_property_discovery_uses_schema_format() {
    assert_eq!(
        binary_property_name(r#"{"properties":{"archive":{"type":"string","format":"binary"}}}"#)
            .as_deref(),
        Some("archive")
    );
}

#[test]
fn json_body_validation_resolves_components_before_encoding() {
    let spec = find_operation(ServiceKind::Sonarr, "post_series").unwrap();
    let args = json!({"body": {"title": 7}});
    let error = match encode_request_body(ServiceKind::Sonarr, spec, args.as_object().unwrap()) {
        Ok(_) => panic!("invalid body should fail validation"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("JSON body"), "{error}");
    assert!(error.contains("/title"), "{error}");
}

#[test]
fn json_body_validation_accepts_openapi_nullable_fields() {
    let spec = find_operation(ServiceKind::Sonarr, "post_series").unwrap();
    let args = json!({"body": {"title": null}});
    let encoded = encode_request_body(ServiceKind::Sonarr, spec, args.as_object().unwrap())
        .unwrap()
        .unwrap();
    assert!(matches!(encoded, EncodedRequestBody::Json { .. }));
}

#[test]
fn servarr_command_bodies_accept_command_specific_root_fields() {
    for (kind, body) in [
        (
            ServiceKind::Sonarr,
            json!({"name": "RescanSeries", "seriesId": 7}),
        ),
        (
            ServiceKind::Radarr,
            json!({"name": "RescanMovie", "movieId": 8}),
        ),
    ] {
        let spec = find_operation(kind, "post_command").unwrap();
        let args = json!({"body": body});
        let encoded = encode_request_body(kind, spec, args.as_object().unwrap())
            .unwrap()
            .unwrap();
        assert!(matches!(encoded, EncodedRequestBody::Json { .. }));
    }
}

#[test]
fn servarr_command_accommodation_still_validates_known_fields() {
    let spec = find_operation(ServiceKind::Sonarr, "post_command").unwrap();
    let args = json!({"body": {"name": 7, "seriesId": 8}});
    let error = match encode_request_body(ServiceKind::Sonarr, spec, args.as_object().unwrap()) {
        Ok(_) => panic!("known CommandResource fields must remain validated"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("/name"), "{error}");
    assert!(error.contains("string"), "{error}");
}

#[test]
fn unrelated_closed_body_schemas_still_reject_unknown_fields() {
    let spec = find_operation(ServiceKind::Sonarr, "post_customfilter").unwrap();
    let args = json!({"body": {"commandSpecific": true}});
    let error = match encode_request_body(ServiceKind::Sonarr, spec, args.as_object().unwrap()) {
        Ok(_) => panic!("closed non-command schema must reject unknown fields"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.to_ascii_lowercase().contains("additional properties"),
        "{error}"
    );
}
