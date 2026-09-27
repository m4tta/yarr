//! Code Mode metadata for multipart operation arguments.

use serde::Serialize;
use serde_json::Value;

use crate::openapi::{BodyEncoding, RepresentationSpec, RequestBodySpec};

use super::{CatalogRepresentation, catalog_representation, schema_json};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CatalogMultipart {
    file_fields: Vec<CatalogMultipartFile>,
    controls: Vec<CatalogControl>,
    pub(super) required_alternatives: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogMultipartFile {
    name: String,
    media_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogControl {
    name: &'static str,
    schema: Value,
}

pub(super) fn request_representation(
    representation: &RepresentationSpec,
    multipart: Option<&CatalogMultipart>,
) -> CatalogRepresentation {
    let mut result = catalog_representation(representation);
    if representation.encoding == BodyEncoding::Multipart
        && let Some(multipart) = multipart
    {
        strip_binary_body_fields(&mut result.schema, &multipart.file_fields);
    }
    result
}

pub(super) fn metadata(body: RequestBodySpec) -> Option<CatalogMultipart> {
    let representation = body
        .representations
        .iter()
        .find(|representation| representation.encoding == BodyEncoding::Multipart)?;
    let schema = schema_json(representation.schema);
    let properties = schema.get("properties")?.as_object()?;
    let encoding = schema_json(representation.encoding_metadata);
    let file_fields = properties
        .iter()
        .filter(|(_, schema)| schema.get("format").and_then(Value::as_str) == Some("binary"))
        .map(|(name, _)| CatalogMultipartFile {
            name: name.clone(),
            media_type: encoding
                .get(name)
                .and_then(|value| value.get("contentType"))
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_string(),
        })
        .collect::<Vec<_>>();
    if file_fields.is_empty() {
        return None;
    }
    let file_names = file_fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    let required_alternatives = required_alternatives(&schema, &file_names);
    Some(CatalogMultipart {
        controls: vec![
            CatalogControl {
                name: "multipartFileBase64",
                schema: serde_json::json!({
                    "type": "string",
                    "contentEncoding": "base64",
                    "description": "Top-level base64 file bytes; do not put binary data in body."
                }),
            },
            CatalogControl {
                name: "fileName",
                schema: serde_json::json!({
                    "type": "string",
                    "default": "upload.bin"
                }),
            },
            CatalogControl {
                name: "multipartField",
                schema: serde_json::json!({
                    "type": "string",
                    "enum": file_names,
                    "default": file_names.first().copied().unwrap_or("file")
                }),
            },
            CatalogControl {
                name: "contentType",
                schema: serde_json::json!({
                    "type": "string",
                    "const": representation.media_type,
                    "default": representation.media_type
                }),
            },
        ],
        file_fields,
        required_alternatives,
    })
}

fn required_alternatives(schema: &Value, binary_names: &[&str]) -> Vec<Vec<String>> {
    let base = required_names(schema);
    let branches = schema
        .get("anyOf")
        .and_then(Value::as_array)
        .map(|branches| branches.iter().map(required_names).collect::<Vec<_>>())
        .filter(|branches| !branches.is_empty())
        .unwrap_or_else(|| vec![Vec::new()]);
    let mut alternatives = branches
        .into_iter()
        .map(|branch| {
            base.iter()
                .chain(branch.iter())
                .map(|name| {
                    if binary_names.contains(&name.as_str()) {
                        "multipartFileBase64".to_string()
                    } else {
                        format!("body.{name}")
                    }
                })
                .collect::<Vec<_>>()
        })
        .filter(|alternative| !alternative.is_empty())
        .collect::<Vec<_>>();
    for alternative in &mut alternatives {
        alternative.sort();
        alternative.dedup();
    }
    alternatives.sort();
    alternatives.dedup();
    alternatives
}

fn required_names(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn strip_binary_body_fields(schema: &mut Value, file_fields: &[CatalogMultipartFile]) {
    let names = file_fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        properties.retain(|name, _| !names.contains(&name.as_str()));
    }
    strip_required_binary_names(object, "required", &names);
    if let Some(branches) = object.get_mut("anyOf").and_then(Value::as_array_mut) {
        for branch in branches.iter_mut().filter_map(Value::as_object_mut) {
            strip_required_binary_names(branch, "required", &names);
        }
        branches.retain(|branch| {
            branch
                .get("required")
                .and_then(Value::as_array)
                .is_some_and(|required| !required.is_empty())
        });
    }
}

fn strip_required_binary_names(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    binary_names: &[&str],
) {
    if let Some(required) = object.get_mut(key).and_then(Value::as_array_mut) {
        required.retain(|name| {
            name.as_str()
                .is_none_or(|name| !binary_names.contains(&name))
        });
        if required.is_empty() {
            object.remove(key);
        }
    }
}
