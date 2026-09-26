//! Request-side OpenAPI schema validation.
//!
//! Generated operation rows retain their parameter and representation schemas,
//! while local `$ref`s still point at the vendored document's component table.
//! A validator map is built once per service kind so every request can validate
//! against the original component graph without copying it into generated Rust.

use std::{collections::HashMap, sync::OnceLock};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::{
    config::ServiceKind,
    openapi::{BodyEncoding, operations_for_kind},
};

const MAX_VALIDATION_ERRORS: usize = 3;
const MAX_ERROR_CHARS: usize = 300;
const COMMAND_RESOURCE_SCHEMA: &str = r##"{"$ref":"#/components/schemas/CommandResource"}"##;

struct ServiceValidators {
    validators: jsonschema::ValidatorMap,
    schema_locations: HashMap<&'static str, String>,
}

type CachedValidators = Result<ServiceValidators, String>;

static SONARR: OnceLock<CachedValidators> = OnceLock::new();
static RADARR: OnceLock<CachedValidators> = OnceLock::new();
static PROWLARR: OnceLock<CachedValidators> = OnceLock::new();
static OVERSEERR: OnceLock<CachedValidators> = OnceLock::new();
static JELLYFIN: OnceLock<CachedValidators> = OnceLock::new();
static PLEX: OnceLock<CachedValidators> = OnceLock::new();

pub(super) fn validate_schema(
    kind: ServiceKind,
    schema: &'static str,
    instance: &Value,
    subject: &str,
) -> Result<()> {
    validate_schema_with_policy(kind, schema, instance, subject, false)
}

pub(super) fn validate_json_body_schema(
    kind: ServiceKind,
    operation: &str,
    schema: &'static str,
    instance: &Value,
    subject: &str,
) -> Result<()> {
    // Servarr command creation is polymorphic on `name`: real command payloads
    // add fields such as `seriesId` and `movieId`, but the published generic
    // CommandResource closes the object without describing those variants.
    // Ignore only that root-level closure for these two operations; every known
    // CommandResource property and every nested schema remains validated.
    let allow_root_additional_properties =
        matches!(kind, ServiceKind::Sonarr | ServiceKind::Radarr)
            && operation == "post_command"
            && schema == COMMAND_RESOURCE_SCHEMA;
    validate_schema_with_policy(
        kind,
        schema,
        instance,
        subject,
        allow_root_additional_properties,
    )
}

fn validate_schema_with_policy(
    kind: ServiceKind,
    schema: &'static str,
    instance: &Value,
    subject: &str,
    allow_root_additional_properties: bool,
) -> Result<()> {
    if schema == "null" {
        return Ok(());
    }
    let validators = validators_for(kind)?;
    if let Some(location) = validators.schema_locations.get(schema) {
        let validator = validators
            .validators
            .get(location)
            .ok_or_else(|| anyhow!("{subject}: compiled request schema is unavailable"))?;
        return validate_instance(
            validator,
            instance,
            subject,
            allow_root_additional_properties,
        );
    }

    // Production operation rows are all indexed above. Focused transport tests
    // also exercise hand-built OperationSpecs, so keep that test seam useful.
    let validator = compile_standalone(kind, schema, subject)?;
    validate_instance(
        &validator,
        instance,
        subject,
        allow_root_additional_properties,
    )
}

fn validate_instance(
    validator: &jsonschema::Validator,
    instance: &Value,
    subject: &str,
    allow_root_additional_properties: bool,
) -> Result<()> {
    let mut errors = validator.iter_errors(instance).filter(|error| {
        !(allow_root_additional_properties
            && error.instance_path().as_str().is_empty()
            && matches!(
                error.kind(),
                jsonschema::error::ValidationErrorKind::AdditionalProperties { .. }
            ))
    });
    let Some(first) = errors.next() else {
        return Ok(());
    };
    let mut messages = vec![render_error(&first)];
    messages.extend(
        errors
            .by_ref()
            .take(MAX_VALIDATION_ERRORS)
            .map(|error| render_error(&error)),
    );
    let truncated = messages.len() > MAX_VALIDATION_ERRORS;
    messages.truncate(MAX_VALIDATION_ERRORS);
    if truncated {
        messages.push("additional schema errors omitted".to_string());
    }
    bail!(
        "{subject} failed schema validation: {}",
        messages.join("; ")
    )
}

fn compile_standalone(
    kind: ServiceKind,
    source: &'static str,
    subject: &str,
) -> Result<jsonschema::Validator> {
    let mut request_schema: Value =
        serde_json::from_str(source).with_context(|| format!("{subject}: parse request schema"))?;
    normalize_openapi_schema(&mut request_schema);
    let spec = parse_vendored_spec(kind)?;
    let mut components = spec
        .get("components")
        .and_then(|value| value.get("schemas"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    normalize_openapi_schema(&mut components);
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "allOf": [request_schema],
        "components": { "schemas": components },
    });
    jsonschema::validator_for(&schema).with_context(|| format!("{subject}: compile request schema"))
}

fn validators_for(kind: ServiceKind) -> Result<&'static ServiceValidators> {
    let cache = match kind {
        ServiceKind::Sonarr => &SONARR,
        ServiceKind::Radarr => &RADARR,
        ServiceKind::Prowlarr => &PROWLARR,
        ServiceKind::Overseerr => &OVERSEERR,
        ServiceKind::Jellyfin => &JELLYFIN,
        ServiceKind::Plex => &PLEX,
        _ => bail!("{} has no generated OpenAPI schema", kind.as_str()),
    };
    cache
        .get_or_init(|| build_validators(kind).map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|message| anyhow!(message.clone()))
}

fn build_validators(kind: ServiceKind) -> Result<ServiceValidators> {
    let spec = parse_vendored_spec(kind)?;
    let mut components = spec
        .get("components")
        .and_then(|value| value.get("schemas"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    normalize_openapi_schema(&mut components);

    let mut definitions = Map::new();
    let mut schema_locations = HashMap::new();
    for operation in operations_for_kind(kind) {
        for parameter in operation.parameters {
            index_schema(parameter.schema, &mut definitions, &mut schema_locations)?;
        }
        if let Some(body) = operation.request_body {
            for representation in body
                .representations
                .iter()
                .filter(|representation| representation.encoding == BodyEncoding::Json)
            {
                index_schema(
                    representation.schema,
                    &mut definitions,
                    &mut schema_locations,
                )?;
            }
        }
    }

    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": definitions,
        "components": { "schemas": components },
    });
    let validators = jsonschema::validator_map_for(&schema)
        .with_context(|| format!("compile {} request schemas", kind.as_str()))?;
    Ok(ServiceValidators {
        validators,
        schema_locations,
    })
}

fn index_schema(
    source: &'static str,
    definitions: &mut Map<String, Value>,
    locations: &mut HashMap<&'static str, String>,
) -> Result<()> {
    if source == "null" || locations.contains_key(source) {
        return Ok(());
    }
    let mut schema: Value =
        serde_json::from_str(source).context("parse generated request schema")?;
    normalize_openapi_schema(&mut schema);
    let name = format!("request_{}", definitions.len());
    locations.insert(source, format!("#/$defs/{name}"));
    definitions.insert(name, schema);
    Ok(())
}

fn parse_vendored_spec(kind: ServiceKind) -> Result<Value> {
    let (source, yaml) = match kind {
        ServiceKind::Sonarr => (include_str!("../../../specs/sonarr.openapi.json"), false),
        ServiceKind::Radarr => (include_str!("../../../specs/radarr.openapi.json"), false),
        ServiceKind::Prowlarr => (include_str!("../../../specs/prowlarr.openapi.json"), false),
        ServiceKind::Overseerr => (include_str!("../../../specs/overseerr.openapi.yml"), true),
        ServiceKind::Jellyfin => (include_str!("../../../specs/jellyfin.openapi.json"), false),
        ServiceKind::Plex => (include_str!("../../../specs/plex.openapi.yml"), true),
        _ => bail!("{} has no vendored OpenAPI document", kind.as_str()),
    };
    if yaml {
        noyalib::from_str_strict(source)
            .with_context(|| format!("parse vendored {} OpenAPI YAML", kind.as_str()))
    } else {
        serde_json::from_str(source)
            .with_context(|| format!("parse vendored {} OpenAPI JSON", kind.as_str()))
    }
}

/// OpenAPI 3.0 expresses nullability with `nullable: true`, which JSON Schema
/// validators do not interpret. Rewrite it without changing any constraints.
fn normalize_openapi_schema(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let nullable =
                object.remove("nullable").and_then(|value| value.as_bool()) == Some(true);
            for child in object.values_mut() {
                normalize_openapi_schema(child);
            }
            if nullable {
                match object.get("type").cloned() {
                    Some(Value::String(value)) => {
                        object.insert("type".to_string(), json!([value, "null"]));
                    }
                    Some(Value::Array(mut values)) => {
                        if !values.iter().any(|value| value == "null") {
                            values.push(json!("null"));
                        }
                        object.insert("type".to_string(), Value::Array(values));
                    }
                    _ => {
                        let inner = std::mem::take(object);
                        object.insert("anyOf".to_string(), json!([inner, { "type": "null" }]));
                    }
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_openapi_schema(value);
            }
        }
        _ => {}
    }
}

fn render_error(error: &jsonschema::ValidationError<'_>) -> String {
    let path = error.instance_path().as_str();
    let location = if path.is_empty() { "/" } else { path };
    truncate(
        &format!("at {location}: {}", error.masked()),
        MAX_ERROR_CHARS,
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
