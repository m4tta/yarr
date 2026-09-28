//! DownloadClient capability: business-layer methods for SABnzbd (C5) and
//! qBittorrent (C5).
//!
//! The two clients share a verb set (`queue`, `add`, `pause`, `resume`,
//! `remove`) but their APIs diverge *completely*: SABnzbd is a `?mode=` query API
//! authed by an `apikey` query param, while qBittorrent is a `/api/v2` REST API
//! authed by a username/password cookie session. Per the locked bead decision the
//! per-client split is **UNCONDITIONAL** — each public method on
//! [`YarrService`] resolves the service's
//! [`KindDescriptor`](crate::capability::KindDescriptor) and dispatches to the
//! [`sab`] or [`qbit`] impl by `query_api` flag rather than matching the kind
//! ad-hoc inside the method body.
//!
//! Scope split (locked in the bead): `queue` is READ; `add`, `pause`, `resume`,
//! and `remove` mutate, so they are WRITE. Only `remove` is *destructive* (it
//! deletes a download, optionally its data). That classification is informative;
//! CLI, MCP, and Code Mode run it immediately after authorization and validation.
//! `remove` defaults `delete_files` to `false` (opt-in via `--delete-files` /
//! `delete_files=true`).
//!
//! The curated-command *descriptors* (registry table) live in
//! `src/actions/commands/download.rs`, not here — this module only holds logic.

pub mod qbit;
pub mod sab;

use anyhow::Result;
use serde_json::Value;

use crate::app::YarrService;
use crate::capability::Capability;
use crate::config::{ServiceConfig, ServiceKind};

impl YarrService {
    /// Resolve a DownloadClient service and verify its capability. Central helper
    /// so every download method shares one capability-checked resolution path; a
    /// non-download kind (e.g. plex) is rejected here before any request is built.
    fn download_context<'a>(&'a self, service: &str) -> Result<&'a ServiceConfig> {
        self.service_of_capability(service, Capability::DownloadClient)
    }

    fn qbit_context<'a>(&'a self, service: &str) -> Result<&'a ServiceConfig> {
        let config = self.download_context(service)?;
        if config.kind != ServiceKind::Qbittorrent {
            anyhow::bail!("action is only supported for qBittorrent services");
        }
        Ok(config)
    }

    /// List the active downloads, slimmed. READ.
    ///
    /// Dispatches by `query_api`: SABnzbd (`?mode=queue`) vs qBittorrent
    /// (`/api/v2/torrents/info`).
    pub async fn download_queue(&self, service: &str) -> Result<Value> {
        let config = self.download_context(service)?;
        if config.kind.descriptor().query_api() {
            sab::queue(self, config).await
        } else {
            qbit::queue(self, config).await
        }
    }

    /// Read global qBittorrent connection, throughput, totals, and rate limits.
    pub async fn download_transfer(&self, service: &str) -> Result<Value> {
        qbit::transfer(self, self.qbit_context(service)?).await
    }

    /// Set global limits when no selector is supplied, or limits for exactly one
    /// torrent when `id`/`hash` is supplied. Limits are bytes/s; zero is unlimited.
    pub async fn download_set_limits(
        &self,
        service: &str,
        id: Option<&str>,
        hash: Option<&str>,
        download_limit: Option<i64>,
        upload_limit: Option<i64>,
    ) -> Result<Value> {
        if download_limit.is_none() && upload_limit.is_none() {
            anyhow::bail!("download_set_limits requires download_limit or upload_limit");
        }
        if download_limit.is_some_and(|limit| limit < 0)
            || upload_limit.is_some_and(|limit| limit < 0)
        {
            anyhow::bail!("download_limit and upload_limit must be >= 0 bytes/second");
        }
        let target = exact_qbit_target(id, hash, false)?;
        qbit::set_limits(
            self,
            self.qbit_context(service)?,
            target.as_deref(),
            download_limit,
            upload_limit,
        )
        .await
    }

    pub async fn download_categories(&self, service: &str) -> Result<Value> {
        qbit::categories(self, self.qbit_context(service)?).await
    }

    pub async fn download_create_category(
        &self,
        service: &str,
        category: &str,
        save_path: Option<&str>,
    ) -> Result<Value> {
        let category = exact_category(category)?;
        qbit::create_category(
            self,
            self.qbit_context(service)?,
            category,
            save_path.unwrap_or(""),
        )
        .await
    }

    pub async fn download_edit_category(
        &self,
        service: &str,
        category: &str,
        save_path: &str,
    ) -> Result<Value> {
        let category = exact_category(category)?;
        qbit::edit_category(self, self.qbit_context(service)?, category, save_path).await
    }

    pub async fn download_remove_category(&self, service: &str, category: &str) -> Result<Value> {
        let category = exact_category(category)?;
        qbit::remove_category(self, self.qbit_context(service)?, category).await
    }

    pub async fn download_set_category(
        &self,
        service: &str,
        id: Option<&str>,
        hash: Option<&str>,
        category: Option<&str>,
    ) -> Result<Value> {
        let target = exact_qbit_target(id, hash, true)?
            .ok_or_else(|| anyhow::anyhow!("one of id or hash is required"))?;
        let category = category.map(exact_category).transpose()?.unwrap_or("");
        qbit::set_category(self, self.qbit_context(service)?, &target, category).await
    }

    pub async fn download_tags(&self, service: &str) -> Result<Value> {
        qbit::tags(self, self.qbit_context(service)?).await
    }

    pub async fn download_create_tags(&self, service: &str, tags: &[String]) -> Result<Value> {
        let tags = exact_tags(tags, false)?;
        qbit::create_tags(self, self.qbit_context(service)?, &tags).await
    }

    pub async fn download_delete_tags(&self, service: &str, tags: &[String]) -> Result<Value> {
        let tags = exact_tags(tags, false)?;
        qbit::delete_tags(self, self.qbit_context(service)?, &tags).await
    }

    pub async fn download_add_tags(
        &self,
        service: &str,
        id: Option<&str>,
        hash: Option<&str>,
        tags: &[String],
    ) -> Result<Value> {
        let target = exact_qbit_target(id, hash, true)?
            .ok_or_else(|| anyhow::anyhow!("one of id or hash is required"))?;
        let tags = exact_tags(tags, false)?;
        qbit::add_tags(self, self.qbit_context(service)?, &target, &tags).await
    }

    pub async fn download_remove_tags(
        &self,
        service: &str,
        id: Option<&str>,
        hash: Option<&str>,
        tags: &[String],
    ) -> Result<Value> {
        let target = exact_qbit_target(id, hash, true)?
            .ok_or_else(|| anyhow::anyhow!("one of id or hash is required"))?;
        let tags = exact_tags(tags, true)?;
        qbit::remove_tags(self, self.qbit_context(service)?, &target, &tags).await
    }

    /// Add a download by URL/magnet. Mutating but not destructive; runs
    /// immediately after authorization and validation.
    pub async fn download_add(&self, service: &str, url: &str) -> Result<Value> {
        let config = self.download_context(service)?;
        if config.kind.descriptor().query_api() {
            sab::add(self, config, url).await
        } else {
            qbit::add(self, config, url).await
        }
    }

    /// Pause downloads (all, or a specific id/hash). Mutating but not
    /// destructive; runs immediately after authorization and validation.
    pub async fn download_pause(&self, service: &str, id: Option<&str>) -> Result<Value> {
        let config = self.download_context(service)?;
        if config.kind.descriptor().query_api() {
            sab::pause(self, config, id).await
        } else {
            qbit::pause(self, config, id).await
        }
    }

    /// Resume downloads (all, or a specific id/hash). Mutating but not
    /// destructive; runs immediately after authorization and validation.
    pub async fn download_resume(&self, service: &str, id: Option<&str>) -> Result<Value> {
        let config = self.download_context(service)?;
        if config.kind.descriptor().query_api() {
            sab::resume(self, config, id).await
        } else {
            qbit::resume(self, config, id).await
        }
    }

    /// Remove a download. `delete_files` (default false) also deletes the
    /// downloaded data. Carries informative destructive metadata and runs
    /// immediately after authorization and validation.
    pub async fn download_remove(
        &self,
        service: &str,
        id: &str,
        delete_files: bool,
    ) -> Result<Value> {
        let config = self.download_context(service)?;
        if config.kind.descriptor().query_api() {
            sab::remove(self, config, id, delete_files).await
        } else {
            qbit::remove(self, config, id, delete_files).await
        }
    }
}

fn exact_qbit_target(
    id: Option<&str>,
    hash: Option<&str>,
    required: bool,
) -> Result<Option<String>> {
    let target = match (id, hash) {
        (Some(_), Some(_)) => anyhow::bail!("supply exactly one of id or hash"),
        (Some(value), None) | (None, Some(value)) => Some(value.trim()),
        (None, None) if required => anyhow::bail!("one of id or hash is required"),
        (None, None) => None,
    };
    target
        .map(|value| {
            if value.is_empty()
                || value.eq_ignore_ascii_case("all")
                || value.contains(['|', '\r', '\n'])
            {
                anyhow::bail!("id/hash must identify exactly one torrent");
            }
            Ok(value.to_owned())
        })
        .transpose()
}

fn exact_category(category: &str) -> Result<&str> {
    let category = category.trim();
    if category.is_empty() || category.contains(['\r', '\n']) {
        anyhow::bail!("category must name exactly one category");
    }
    Ok(category)
}

fn exact_tags(tags: &[String], allow_empty: bool) -> Result<String> {
    if tags.is_empty() {
        if allow_empty {
            return Ok(String::new());
        }
        anyhow::bail!("tags must contain at least one tag");
    }
    let mut clean = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || tag.contains([',', '\r', '\n']) {
            anyhow::bail!("each tag must name exactly one tag and may not contain a comma");
        }
        clean.push(tag);
    }
    Ok(clean.join(","))
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
