//! DownloadClient (SABnzbd, qBittorrent) curated command descriptors (C5).
//!
//! The per-capability const slice the registry concatenates at its single
//! extension point (`build_curated_commands`). Each
//! [`CommandDescriptor`] is the SSOT for one curated command — its scope, params,
//! allowed kinds (via `capability` = [`Capability::DownloadClient`], so only
//! SABnzbd + qBittorrent), schema fragment, help line, and handler.
//!
//! ACTION-NAME UNIQUENESS: registry action names are GLOBALLY unique across
//! capabilities, and the ArrManager surface already owns a `queue` command (C1).
//! So these use `download_`-prefixed names (`download_queue`, `download_add`, …);
//! the CLI maps the friendlier kebab verbs (`queue`/`add`/`pause`/`resume`/
//! `remove`) onto them.
//!
//! Handlers are THIN adapters: extract params with the shared parse helpers and
//! call the corresponding `YarrService` method. No business logic here — the
//! per-client path/slim logic lives in `crate::app::download`.

use serde_json::Value;

use crate::actions::model::{READ_SCOPE, WRITE_SCOPE};
use crate::actions::parse::{bool_arg, optional_i64, optional_string, string_arg};
use crate::actions::registry::{
    CommandDescriptor, CommandFuture,
    ParamType::{Boolean, Integer, String as StringParam, StringArray},
};
use crate::app::YarrService;
use crate::capability::Capability;
use crate::config::ServiceKind;

/// Commands backed only by qBittorrent's WebUI API. Registry-derived kind
/// filtering uses this same list for dispatch, MCP schemas, and Code Mode.
pub const QBITTORRENT_ONLY_COMMANDS: &[&str] = &[
    "download_transfer",
    "download_set_limits",
    "download_categories",
    "download_create_category",
    "download_edit_category",
    "download_remove_category",
    "download_set_category",
    "download_tags",
    "download_create_tags",
    "download_delete_tags",
    "download_add_tags",
    "download_remove_tags",
];

pub fn command_supports_kind(action: &str, kind: ServiceKind) -> bool {
    !QBITTORRENT_ONLY_COMMANDS.contains(&action) || kind == ServiceKind::Qbittorrent
}

/// The DownloadClient (SABnzbd, qBittorrent) curated commands.
pub const DOWNLOAD_COMMANDS: &[CommandDescriptor] = &[
    CommandDescriptor {
        name: "download_queue",
        capability: Capability::DownloadClient,
        description: "list the active downloads (sab queue slots / qbit torrents), slimmed.",
        required_scope: READ_SCOPE,
        required_params: &["service"],
        optional_params: &[],
        destructive: false,
        mutates: false,
        typed_params: &[],
        handler: handle_queue,
    },
    CommandDescriptor {
        name: "download_add",
        capability: Capability::DownloadClient,
        description: "queue a new download from a --url/magnet (write). Non-destructive — \
             runs immediately.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "url"],
        optional_params: &[],
        destructive: false,
        mutates: true,
        typed_params: &[("url", StringParam)],
        handler: handle_add,
    },
    CommandDescriptor {
        name: "download_pause",
        capability: Capability::DownloadClient,
        description: "pause a download by --id/--hash, or all when omitted (write). \
             Non-destructive — runs immediately.",
        required_scope: WRITE_SCOPE,
        required_params: &["service"],
        optional_params: &["id", "hash"],
        destructive: false,
        mutates: true,
        typed_params: &[("id", StringParam), ("hash", StringParam)],
        handler: handle_pause,
    },
    CommandDescriptor {
        name: "download_resume",
        capability: Capability::DownloadClient,
        description: "resume a download by --id/--hash, or all when omitted (write). \
             Non-destructive — runs immediately.",
        required_scope: WRITE_SCOPE,
        required_params: &["service"],
        optional_params: &["id", "hash"],
        destructive: false,
        mutates: true,
        typed_params: &[("id", StringParam), ("hash", StringParam)],
        handler: handle_resume,
    },
    CommandDescriptor {
        name: "download_remove",
        capability: Capability::DownloadClient,
        description: "remove a download by --id/--hash; --delete-files also deletes data \
             (default off). DESTRUCTIVE — on MCP the connected client is elicited for \
             confirmation before this runs.",
        required_scope: WRITE_SCOPE,
        required_params: &["service"],
        optional_params: &["id", "hash", "delete_files"],
        destructive: true,
        mutates: true,
        typed_params: &[
            ("id", StringParam),
            ("hash", StringParam),
            ("delete_files", Boolean),
        ],
        handler: handle_remove,
    },
    CommandDescriptor {
        name: "download_transfer",
        capability: Capability::DownloadClient,
        description: "read qBittorrent global connection, throughput, totals, and rate limits.",
        required_scope: READ_SCOPE,
        required_params: &["service"],
        optional_params: &[],
        destructive: false,
        mutates: false,
        typed_params: &[],
        handler: handle_transfer,
    },
    CommandDescriptor {
        name: "download_set_limits",
        capability: Capability::DownloadClient,
        description: "set qBittorrent global or single-torrent byte/second limits; zero is unlimited.",
        required_scope: WRITE_SCOPE,
        required_params: &["service"],
        optional_params: &["id", "hash", "download_limit", "upload_limit"],
        destructive: false,
        mutates: true,
        typed_params: &[
            ("id", StringParam),
            ("hash", StringParam),
            ("download_limit", Integer),
            ("upload_limit", Integer),
        ],
        handler: handle_set_limits,
    },
    CommandDescriptor {
        name: "download_categories",
        capability: Capability::DownloadClient,
        description: "list qBittorrent categories and save paths.",
        required_scope: READ_SCOPE,
        required_params: &["service"],
        optional_params: &[],
        destructive: false,
        mutates: false,
        typed_params: &[],
        handler: handle_categories,
    },
    CommandDescriptor {
        name: "download_create_category",
        capability: Capability::DownloadClient,
        description: "create a qBittorrent category, optionally with a save path.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "category"],
        optional_params: &["save_path"],
        destructive: false,
        mutates: true,
        typed_params: &[("category", StringParam), ("save_path", StringParam)],
        handler: handle_create_category,
    },
    CommandDescriptor {
        name: "download_edit_category",
        capability: Capability::DownloadClient,
        description: "change a qBittorrent category save path.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "category", "save_path"],
        optional_params: &[],
        destructive: false,
        mutates: true,
        typed_params: &[("category", StringParam), ("save_path", StringParam)],
        handler: handle_edit_category,
    },
    CommandDescriptor {
        name: "download_remove_category",
        capability: Capability::DownloadClient,
        description: "remove exactly one qBittorrent category definition.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "category"],
        optional_params: &[],
        destructive: false,
        mutates: true,
        typed_params: &[("category", StringParam)],
        handler: handle_remove_category,
    },
    CommandDescriptor {
        name: "download_set_category",
        capability: Capability::DownloadClient,
        description: "assign one qBittorrent torrent to a category; omit category to clear it.",
        required_scope: WRITE_SCOPE,
        required_params: &["service"],
        optional_params: &["id", "hash", "category"],
        destructive: false,
        mutates: true,
        typed_params: &[
            ("id", StringParam),
            ("hash", StringParam),
            ("category", StringParam),
        ],
        handler: handle_set_category,
    },
    CommandDescriptor {
        name: "download_tags",
        capability: Capability::DownloadClient,
        description: "list qBittorrent tag definitions.",
        required_scope: READ_SCOPE,
        required_params: &["service"],
        optional_params: &[],
        destructive: false,
        mutates: false,
        typed_params: &[],
        handler: handle_tags,
    },
    CommandDescriptor {
        name: "download_create_tags",
        capability: Capability::DownloadClient,
        description: "create one or more qBittorrent tags.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "tags"],
        optional_params: &[],
        destructive: false,
        mutates: true,
        typed_params: &[("tags", StringArray)],
        handler: handle_create_tags,
    },
    CommandDescriptor {
        name: "download_delete_tags",
        capability: Capability::DownloadClient,
        description: "delete one or more qBittorrent tag definitions.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "tags"],
        optional_params: &[],
        destructive: false,
        mutates: true,
        typed_params: &[("tags", StringArray)],
        handler: handle_delete_tags,
    },
    CommandDescriptor {
        name: "download_add_tags",
        capability: Capability::DownloadClient,
        description: "add tags to exactly one qBittorrent torrent.",
        required_scope: WRITE_SCOPE,
        required_params: &["service", "tags"],
        optional_params: &["id", "hash"],
        destructive: false,
        mutates: true,
        typed_params: &[
            ("id", StringParam),
            ("hash", StringParam),
            ("tags", StringArray),
        ],
        handler: handle_add_tags,
    },
    CommandDescriptor {
        name: "download_remove_tags",
        capability: Capability::DownloadClient,
        description: "remove selected or all tags from exactly one qBittorrent torrent.",
        required_scope: WRITE_SCOPE,
        required_params: &["service"],
        optional_params: &["id", "hash", "tags"],
        destructive: false,
        mutates: true,
        typed_params: &[
            ("id", StringParam),
            ("hash", StringParam),
            ("tags", StringArray),
        ],
        handler: handle_remove_tags,
    },
];

// ── thin handler adapters (marshal params → service method) ──────────────────────

fn handle_queue<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let service = string_arg(args, "service")?;
        svc.download_queue(&service).await
    })
}

fn handle_add<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let service = string_arg(args, "service")?;
        let url = string_arg(args, "url")?;
        svc.download_add(&service, &url).await
    })
}

fn handle_pause<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let service = string_arg(args, "service")?;
        let id = download_id(args)?;
        svc.download_pause(&service, id.as_deref()).await
    })
}

fn handle_resume<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let service = string_arg(args, "service")?;
        let id = download_id(args)?;
        svc.download_resume(&service, id.as_deref()).await
    })
}

fn handle_remove<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let service = string_arg(args, "service")?;
        let id = download_id(args)?.ok_or_else(|| {
            crate::actions::model::ValidationError::MissingField { field: "id".into() }
        })?;
        svc.download_remove(&service, &id, bool_arg(args, "delete_files")?)
            .await
    })
}

fn handle_transfer<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move { svc.download_transfer(&string_arg(args, "service")?).await })
}

fn handle_set_limits<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let (id, hash) = selectors(args)?;
        svc.download_set_limits(
            &string_arg(args, "service")?,
            id.as_deref(),
            hash.as_deref(),
            optional_i64(args, "download_limit")?,
            optional_i64(args, "upload_limit")?,
        )
        .await
    })
}

fn handle_categories<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move { svc.download_categories(&string_arg(args, "service")?).await })
}

fn handle_create_category<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        svc.download_create_category(
            &string_arg(args, "service")?,
            &string_arg(args, "category")?,
            optional_string(args, "save_path")?.as_deref(),
        )
        .await
    })
}

fn handle_edit_category<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        svc.download_edit_category(
            &string_arg(args, "service")?,
            &string_arg(args, "category")?,
            &string_arg(args, "save_path")?,
        )
        .await
    })
}

fn handle_remove_category<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        svc.download_remove_category(
            &string_arg(args, "service")?,
            &string_arg(args, "category")?,
        )
        .await
    })
}

fn handle_set_category<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let (id, hash) = selectors(args)?;
        svc.download_set_category(
            &string_arg(args, "service")?,
            id.as_deref(),
            hash.as_deref(),
            optional_string(args, "category")?.as_deref(),
        )
        .await
    })
}

fn handle_tags<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move { svc.download_tags(&string_arg(args, "service")?).await })
}

fn handle_create_tags<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        svc.download_create_tags(&string_arg(args, "service")?, &string_list(args, "tags")?)
            .await
    })
}

fn handle_delete_tags<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        svc.download_delete_tags(&string_arg(args, "service")?, &string_list(args, "tags")?)
            .await
    })
}

fn handle_add_tags<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let (id, hash) = selectors(args)?;
        svc.download_add_tags(
            &string_arg(args, "service")?,
            id.as_deref(),
            hash.as_deref(),
            &string_list(args, "tags")?,
        )
        .await
    })
}

fn handle_remove_tags<'a>(svc: &'a YarrService, args: &'a Value) -> CommandFuture<'a> {
    Box::pin(async move {
        let (id, hash) = selectors(args)?;
        svc.download_remove_tags(
            &string_arg(args, "service")?,
            id.as_deref(),
            hash.as_deref(),
            &optional_string_list(args, "tags")?,
        )
        .await
    })
}

/// The download identifier: SABnzbd uses `nzo_id`, qBittorrent uses `hash`. Both
/// are exposed via `--id` (canonical) and `--hash` (qbit-friendly alias); either
/// param resolves to the same value here.
fn download_id(args: &Value) -> anyhow::Result<Option<String>> {
    Ok(optional_string(args, "id")?.or(optional_string(args, "hash")?))
}

fn selectors(args: &Value) -> anyhow::Result<(Option<String>, Option<String>)> {
    Ok((optional_string(args, "id")?, optional_string(args, "hash")?))
}

fn string_list(args: &Value, field: &str) -> anyhow::Result<Vec<String>> {
    match args.get(field) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    crate::actions::model::ValidationError::WrongType {
                        field: field.to_owned(),
                    }
                    .into()
                })
            })
            .collect(),
        Some(_) => Err(crate::actions::model::ValidationError::WrongType {
            field: field.to_owned(),
        }
        .into()),
        None => Err(crate::actions::model::ValidationError::MissingField {
            field: field.to_owned(),
        }
        .into()),
    }
}

fn optional_string_list(args: &Value, field: &str) -> anyhow::Result<Vec<String>> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(_) => string_list(args, field),
    }
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
