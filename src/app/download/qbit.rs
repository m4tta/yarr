//! qBittorrent DownloadClient impl (C5).
//!
//! qBittorrent's WebUI API is a `/api/v2` REST surface authed by a username/
//! password SID cookie (established in [`crate::yarr::auth`], a dedicated
//! cookie-store client per the F2/S1 isolation fix). Reads are GETs; mutations
//! are `application/x-www-form-urlencoded` POSTs through
//! [`send_form_post`](crate::yarr::YarrClient::send_form_post), which
//! percent-encodes every field — callers never `format!` values into the body.
//!
//! API-VERSION FACT (bead, HIGH): qBittorrent **v5 renamed** the pause/resume
//! endpoints — `pause` → `POST /api/v2/torrents/stop`, `resume` →
//! `POST /api/v2/torrents/start`. The v4 `pause`/`resume` paths are GONE, so this
//! module targets the v5 `stop`/`start` names.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::app::YarrService;
use crate::config::ServiceConfig;
use crate::yarr::slim;

/// Envelope for a qBittorrent bulk mutation (`stop`/`start`/`delete`).
///
/// qBittorrent returns HTTP 200 with an EMPTY body even when the supplied hashes
/// match NO torrent, and the transport coerces an empty body to `{ok:true,...}`.
/// Returning that bare value would falsely imply the target existed. Instead we
/// return a `submitted` envelope that makes clear the action was accepted but NOT
/// confirmed against a real torrent, and points the caller at `queue` to verify.
fn qbit_submitted(status: u16) -> Value {
    json!({
        "submitted": true,
        "status": status,
        "note": "qBittorrent returns no confirmation body; verify with `queue`",
    })
}

/// Read the HTTP status the transport stamped onto a coerced empty-body response
/// (`{ok:true,status:200}`), defaulting to 200 when absent.
fn response_status(response: &Value) -> u16 {
    response
        .get("status")
        .and_then(Value::as_u64)
        .map(|s| s as u16)
        .unwrap_or(200)
}

/// Fields kept for a slimmed `/torrents/info` row — identify a torrent and reason
/// about its progress/throughput without the (large) full payload.
const TORRENT_FIELDS: &[&str] = &[
    "hash", "name", "state", "progress", "dlspeed", "upspeed", "eta", "size", "category", "tags",
    "dl_limit", "up_limit",
];

const TRANSFER_FIELDS: &[&str] = &[
    "connection_status",
    "dl_info_speed",
    "up_info_speed",
    "dl_info_data",
    "up_info_data",
    "dl_rate_limit",
    "up_rate_limit",
];

/// Build `{api_prefix}{suffix}` for the qBittorrent service (descriptor-driven —
/// `/api/v2`, no hardcoded version). Pure for testability.
pub(super) fn qbit_path(config: &ServiceConfig, suffix: &str) -> String {
    format!("{}{}", config.kind.descriptor().api_prefix, suffix)
}

/// GET `/api/v2/torrents/info` → active torrents, slimmed to [`TORRENT_FIELDS`].
pub(super) async fn queue(svc: &YarrService, config: &ServiceConfig) -> Result<Value> {
    let path = qbit_path(config, "/torrents/info");
    let url = crate::yarr::build_url(config, &path)?;
    let raw = svc.client_ref().send_get(config, url, None).await?;
    Ok(slim(raw, TORRENT_FIELDS))
}

/// GET `/api/v2/transfer/info` → bounded global transfer status and limits.
pub(super) async fn transfer(svc: &YarrService, config: &ServiceConfig) -> Result<Value> {
    let path = qbit_path(config, "/transfer/info");
    let url = crate::yarr::build_url(config, &path)?;
    let raw = svc.client_ref().send_get(config, url, None).await?;
    Ok(slim(raw, TRANSFER_FIELDS))
}

/// Set one or both global/per-torrent byte-per-second limits. The caller has
/// already validated both limits before this function performs its first POST.
pub(super) async fn set_limits(
    svc: &YarrService,
    config: &ServiceConfig,
    target: Option<&str>,
    download_limit: Option<i64>,
    upload_limit: Option<i64>,
) -> Result<Value> {
    let target_name = target.unwrap_or("global");
    if let Some(limit) = download_limit {
        let value = limit.to_string();
        let (suffix, form) = match target {
            Some(hash) => (
                "/torrents/setDownloadLimit",
                vec![("hashes", hash), ("limit", value.as_str())],
            ),
            None => (
                "/transfer/setDownloadLimit",
                vec![("limit", value.as_str())],
            ),
        };
        post_form(svc, config, suffix, &form).await?;
    }
    if let Some(limit) = upload_limit {
        let value = limit.to_string();
        let (suffix, form) = match target {
            Some(hash) => (
                "/torrents/setUploadLimit",
                vec![("hashes", hash), ("limit", value.as_str())],
            ),
            None => ("/transfer/setUploadLimit", vec![("limit", value.as_str())]),
        };
        post_form(svc, config, suffix, &form)
            .await
            .with_context(|| {
                if download_limit.is_some() {
                    "download limit was applied, but setting upload limit failed"
                } else {
                    "setting upload limit failed"
                }
            })?;
    }
    Ok(json!({
        "submitted": true,
        "target": target_name,
        "downloadLimit": download_limit,
        "uploadLimit": upload_limit,
        "unit": "bytes_per_second",
    }))
}

pub(super) async fn categories(svc: &YarrService, config: &ServiceConfig) -> Result<Value> {
    get(svc, config, "/torrents/categories").await
}

pub(super) async fn create_category(
    svc: &YarrService,
    config: &ServiceConfig,
    category: &str,
    save_path: &str,
) -> Result<Value> {
    post_form(
        svc,
        config,
        "/torrents/createCategory",
        &[("category", category), ("savePath", save_path)],
    )
    .await?;
    Ok(qbit_submitted(200))
}

pub(super) async fn edit_category(
    svc: &YarrService,
    config: &ServiceConfig,
    category: &str,
    save_path: &str,
) -> Result<Value> {
    post_form(
        svc,
        config,
        "/torrents/editCategory",
        &[("category", category), ("savePath", save_path)],
    )
    .await?;
    Ok(qbit_submitted(200))
}

pub(super) async fn remove_category(
    svc: &YarrService,
    config: &ServiceConfig,
    category: &str,
) -> Result<Value> {
    post_form(
        svc,
        config,
        "/torrents/removeCategories",
        &[("categories", category)],
    )
    .await?;
    Ok(qbit_submitted(200))
}

pub(super) async fn set_category(
    svc: &YarrService,
    config: &ServiceConfig,
    hash: &str,
    category: &str,
) -> Result<Value> {
    post_form(
        svc,
        config,
        "/torrents/setCategory",
        &[("hashes", hash), ("category", category)],
    )
    .await?;
    Ok(qbit_submitted(200))
}

pub(super) async fn tags(svc: &YarrService, config: &ServiceConfig) -> Result<Value> {
    get(svc, config, "/torrents/tags").await
}

pub(super) async fn create_tags(
    svc: &YarrService,
    config: &ServiceConfig,
    tags: &str,
) -> Result<Value> {
    tags_post(svc, config, "/torrents/createTags", None, tags).await
}

pub(super) async fn delete_tags(
    svc: &YarrService,
    config: &ServiceConfig,
    tags: &str,
) -> Result<Value> {
    tags_post(svc, config, "/torrents/deleteTags", None, tags).await
}

pub(super) async fn add_tags(
    svc: &YarrService,
    config: &ServiceConfig,
    hash: &str,
    tags: &str,
) -> Result<Value> {
    tags_post(svc, config, "/torrents/addTags", Some(hash), tags).await
}

pub(super) async fn remove_tags(
    svc: &YarrService,
    config: &ServiceConfig,
    hash: &str,
    tags: &str,
) -> Result<Value> {
    tags_post(svc, config, "/torrents/removeTags", Some(hash), tags).await
}

async fn get(svc: &YarrService, config: &ServiceConfig, suffix: &str) -> Result<Value> {
    let path = qbit_path(config, suffix);
    let url = crate::yarr::build_url(config, &path)?;
    svc.client_ref().send_get(config, url, None).await
}

async fn post_form(
    svc: &YarrService,
    config: &ServiceConfig,
    suffix: &str,
    form: &[(&str, &str)],
) -> Result<Value> {
    let path = qbit_path(config, suffix);
    let url = crate::yarr::build_url(config, &path)?;
    svc.client_ref().send_form_post(config, url, form).await
}

async fn tags_post(
    svc: &YarrService,
    config: &ServiceConfig,
    suffix: &str,
    hash: Option<&str>,
    tags: &str,
) -> Result<Value> {
    let form = match hash {
        Some(hash) => vec![("hashes", hash), ("tags", tags)],
        None => vec![("tags", tags)],
    };
    post_form(svc, config, suffix, &form).await?;
    Ok(qbit_submitted(200))
}

/// POST `/api/v2/torrents/add` (form field `urls`) → add a download by URL/magnet.
pub(super) async fn add(svc: &YarrService, config: &ServiceConfig, url: &str) -> Result<Value> {
    let path = qbit_path(config, "/torrents/add");
    let request = crate::yarr::build_url(config, &path)?;
    svc.client_ref()
        .send_form_post(config, request, &[("urls", url)])
        .await
}

/// POST `/api/v2/torrents/stop` (v5 name — was `pause` in v4) with
/// `hashes=<hash>` or `hashes=all`.
pub(super) async fn pause(
    svc: &YarrService,
    config: &ServiceConfig,
    id: Option<&str>,
) -> Result<Value> {
    let path = qbit_path(config, "/torrents/stop");
    let url = crate::yarr::build_url(config, &path)?;
    let hashes = id.unwrap_or("all");
    let response = svc
        .client_ref()
        .send_form_post(config, url, &[("hashes", hashes)])
        .await?;
    Ok(qbit_submitted(response_status(&response)))
}

/// POST `/api/v2/torrents/start` (v5 name — was `resume` in v4) with
/// `hashes=<hash>` or `hashes=all`.
pub(super) async fn resume(
    svc: &YarrService,
    config: &ServiceConfig,
    id: Option<&str>,
) -> Result<Value> {
    let path = qbit_path(config, "/torrents/start");
    let url = crate::yarr::build_url(config, &path)?;
    let hashes = id.unwrap_or("all");
    let response = svc
        .client_ref()
        .send_form_post(config, url, &[("hashes", hashes)])
        .await?;
    Ok(qbit_submitted(response_status(&response)))
}

/// POST `/api/v2/torrents/delete` with `hashes=<hash>` and
/// `deleteFiles=true|false` (default false — opt-in via `delete_files`).
pub(super) async fn remove(
    svc: &YarrService,
    config: &ServiceConfig,
    id: &str,
    delete_files: bool,
) -> Result<Value> {
    let path = qbit_path(config, "/torrents/delete");
    let url = crate::yarr::build_url(config, &path)?;
    let delete = if delete_files { "true" } else { "false" };
    let response = svc
        .client_ref()
        .send_form_post(config, url, &[("hashes", id), ("deleteFiles", delete)])
        .await?;
    Ok(qbit_submitted(response_status(&response)))
}

#[cfg(test)]
#[path = "qbit_tests.rs"]
mod tests;
