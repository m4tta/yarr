use super::*;
use serde_json::json;

#[test]
fn compiles_request_schemas_for_every_generated_service() {
    for kind in [
        ServiceKind::Sonarr,
        ServiceKind::Radarr,
        ServiceKind::Prowlarr,
        ServiceKind::Overseerr,
        ServiceKind::Jellyfin,
        ServiceKind::Plex,
    ] {
        let validators = validators_for(kind).unwrap_or_else(|error| {
            panic!(
                "{} request schemas did not compile: {error:#}",
                kind.as_str()
            )
        });
        assert!(
            !validators.schema_locations.is_empty(),
            "{} has no indexed request schemas",
            kind.as_str()
        );
    }
}

#[test]
fn validates_primitive_parameter_types_without_coercion() {
    let error = validate_schema(
        ServiceKind::Sonarr,
        r#"{"format":"int32","type":"integer"}"#,
        &json!("7"),
        "operation `get_series_by_id` path parameter `id`",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("path parameter `id`"), "{error}");
    assert!(error.contains("at /"), "{error}");
    assert!(error.contains("integer"), "{error}");
    assert!(
        !error.contains("\"7\""),
        "invalid values must be masked: {error}"
    );
}

#[test]
fn resolves_component_refs_and_reports_the_body_field() {
    let error = validate_schema(
        ServiceKind::Sonarr,
        r##"{"$ref":"#/components/schemas/SeriesResource"}"##,
        &json!({"title": 7}),
        "operation `post_series` JSON body",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("JSON body"), "{error}");
    assert!(error.contains("/title"), "{error}");
    assert!(error.contains("string"), "{error}");
}

#[test]
fn accepts_openapi_nullable_fields_and_refs() {
    validate_schema(
        ServiceKind::Sonarr,
        r##"{"$ref":"#/components/schemas/SeriesResource"}"##,
        &json!({"title": null}),
        "operation `post_series` JSON body",
    )
    .unwrap();

    let mut schema = json!({
        "$ref": "#/components/schemas/SeriesResource",
        "nullable": true
    });
    normalize_openapi_schema(&mut schema);
    assert_eq!(
        schema,
        json!({
            "anyOf": [
                { "$ref": "#/components/schemas/SeriesResource" },
                { "type": "null" }
            ]
        })
    );
    validate_schema(
        ServiceKind::Sonarr,
        r##"{"$ref":"#/components/schemas/SeriesResource","nullable":true}"##,
        &Value::Null,
        "nullable body",
    )
    .unwrap();
}

#[test]
fn validation_errors_are_bounded() {
    let error = validate_schema(
        ServiceKind::Sonarr,
        r##"{"$ref":"#/components/schemas/SeriesResource"}"##,
        &json!({
            "title": 1,
            "year": "bad",
            "monitored": "bad",
            "seasonFolder": "bad",
            "useSceneNumbering": "bad"
        }),
        "body",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("additional schema errors omitted"),
        "{error}"
    );
    assert!(
        error.len() < 1_200,
        "error was not bounded: {} bytes",
        error.len()
    );
}
