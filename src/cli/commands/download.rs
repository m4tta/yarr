//! CLI parse module for DownloadClient curated commands (sabnzbd, qbittorrent).
//!
//! Thin shim: it recognises the curated verbs, maps the friendly kebab CLI verb
//! to the snake_case registry/MCP action name (`queue` → `download_queue` so it
//! does not collide with the ArrManager `queue` command — registry action names
//! are globally unique), assembles the JSON `params` object (the positional
//! `service` plus any flags) into a [`Command::Curated`], and rejects unknown
//! flags. All business logic lives in `crate::app::download`; validation, scope,
//! and dispatch flow through the shared `execute_service_action` path, exactly
//! like the MCP shim.

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use crate::actions::curated_command;
use crate::capability::Capability;
use crate::cli::command::Command;
use crate::cli::parse::reject_args;
use crate::config::ServiceKind;

/// Canonical friendly CLI verb → snake_case registry action name, in declaration
/// order. SSOT for USAGE rendering and the mechanical CLI↔MCP parity test
/// (`tests/parity.rs`). One entry per DownloadClient curated descriptor.
pub const VERBS: &[(&str, &str)] = &[
    ("queue", "download_queue"),
    ("add", "download_add"),
    ("pause", "download_pause"),
    ("resume", "download_resume"),
    ("remove", "download_remove"),
    ("transfer", "download_transfer"),
    ("set-limits", "download_set_limits"),
    ("categories", "download_categories"),
    ("create-category", "download_create_category"),
    ("edit-category", "download_edit_category"),
    ("remove-category", "download_remove_category"),
    ("set-category", "download_set_category"),
    ("tags", "download_tags"),
    ("create-tags", "download_create_tags"),
    ("delete-tags", "download_delete_tags"),
    ("add-tags", "download_add_tags"),
    ("remove-tags", "download_remove_tags"),
];

/// Try to parse `verb [rest]` as a DownloadClient curated command for `kind`.
///
/// Returns `Ok(Some(cmd))` when `verb` is a known download verb, `Ok(None)` when
/// it is not (so the router falls through to its generic passthrough / "unknown
/// command" handling), and `Err` when the verb matched but its flags were
/// invalid.
pub fn parse(kind: ServiceKind, verb: &str, rest: &[String]) -> Result<Option<Command>> {
    // Single verb→action resolution against `VERBS` (the SSOT). `None` => the verb
    // isn't a DownloadClient curated verb, so fall through to the router.
    let Some(action) = resolve(verb)? else {
        return Ok(None);
    };
    if !crate::actions::commands::download::command_supports_kind(action, kind) {
        return Err(anyhow!("`{verb}` is only supported for qbittorrent"));
    }

    // Branch on the PARSING SHAPE only — keyed by the friendly verb, not a second
    // verb→action mapping.
    match verb {
        "queue" => {
            reject_args(rest, verb)?;
            Ok(Some(Command::Curated {
                action,
                params: json!({ "service": kind.as_str() }),
            }))
        }
        "add" => parse_add(kind, action, rest).map(Some),
        "pause" => parse_state(kind, action, "pause", rest).map(Some),
        "resume" => parse_state(kind, action, "resume", rest).map(Some),
        "remove" => parse_remove(kind, action, rest).map(Some),
        "transfer" | "categories" | "tags" => {
            reject_args(rest, verb)?;
            Ok(Some(Command::Curated {
                action,
                params: Value::Object(base_params(kind)),
            }))
        }
        "set-limits" => parse_limits(kind, action, rest).map(Some),
        "create-category" | "edit-category" | "remove-category" | "set-category" => {
            parse_category(kind, action, verb, rest).map(Some)
        }
        "create-tags" | "delete-tags" | "add-tags" | "remove-tags" => {
            parse_tags(kind, action, verb, rest).map(Some)
        }
        // `resolve` only returns `Some` for verbs in `VERBS`; all are handled above.
        _ => unreachable!("VERBS verb `{verb}` has no parse arm"),
    }
}

/// `<svc> add --url X` → `download_add`.
fn parse_add(kind: ServiceKind, action: &'static str, rest: &[String]) -> Result<Command> {
    let mut params = base_params(kind);
    let mut url: Option<String> = None;

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            flag @ ("--url" | "--magnet") => {
                let value = take_value(rest, &mut i, flag)?;
                if url.replace(value).is_some() {
                    return Err(anyhow!("add received duplicate --url"));
                }
            }
            other => return Err(anyhow!("add does not accept argument `{other}`")),
        }
        i += 1;
    }

    let url = url.ok_or_else(|| anyhow!("add requires --url (a URL or magnet link)"))?;
    params.insert("url".into(), json!(url));
    Ok(Command::Curated {
        action,
        params: Value::Object(params),
    })
}

/// `<svc> {pause,resume} [--id N | --hash H]` → `download_{pause,resume}`.
fn parse_state(
    kind: ServiceKind,
    action: &'static str,
    verb: &str,
    rest: &[String],
) -> Result<Command> {
    let mut params = base_params(kind);

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            flag @ ("--id" | "--hash") => {
                let key = if flag == "--hash" { "hash" } else { "id" };
                let value = take_value(rest, &mut i, flag)?;
                if params.insert(key.into(), json!(value)).is_some() {
                    return Err(anyhow!("{verb} received duplicate {flag}"));
                }
            }
            other => return Err(anyhow!("{verb} does not accept argument `{other}`")),
        }
        i += 1;
    }

    Ok(Command::Curated {
        action,
        params: Value::Object(params),
    })
}

/// `<svc> remove (--id N | --hash H) [--delete-files]` → `download_remove`.
fn parse_remove(kind: ServiceKind, action: &'static str, rest: &[String]) -> Result<Command> {
    let mut params = base_params(kind);

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--delete-files" => {
                params.insert("delete_files".into(), json!(true));
            }
            flag @ ("--id" | "--hash") => {
                let key = if flag == "--hash" { "hash" } else { "id" };
                let value = take_value(rest, &mut i, flag)?;
                if params.insert(key.into(), json!(value)).is_some() {
                    return Err(anyhow!("remove received duplicate {flag}"));
                }
            }
            other => return Err(anyhow!("remove does not accept argument `{other}`")),
        }
        i += 1;
    }

    if !params.contains_key("id") && !params.contains_key("hash") {
        return Err(anyhow!(
            "remove requires --id (nzo_id) or --hash (torrent hash)"
        ));
    }
    Ok(Command::Curated {
        action,
        params: Value::Object(params),
    })
}

fn parse_limits(kind: ServiceKind, action: &'static str, rest: &[String]) -> Result<Command> {
    let mut params = base_params(kind);
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            flag @ ("--id" | "--hash") => {
                insert_once(
                    &mut params,
                    &flag[2..],
                    take_value(rest, &mut i, flag)?,
                    flag,
                )?;
            }
            flag @ ("--download-limit" | "--upload-limit") => {
                let value = take_value(rest, &mut i, flag)?
                    .parse::<i64>()
                    .map_err(|_| anyhow!("{flag} requires an integer byte/second value"))?;
                let key = flag[2..].replace('-', "_");
                if params.insert(key, json!(value)).is_some() {
                    return Err(anyhow!("set-limits received duplicate {flag}"));
                }
            }
            other => return Err(anyhow!("set-limits does not accept argument `{other}`")),
        }
        i += 1;
    }
    if !params.contains_key("download_limit") && !params.contains_key("upload_limit") {
        return Err(anyhow!(
            "set-limits requires --download-limit or --upload-limit"
        ));
    }
    reject_both_selectors(&params, "set-limits")?;
    Ok(Command::Curated {
        action,
        params: Value::Object(params),
    })
}

fn parse_category(
    kind: ServiceKind,
    action: &'static str,
    verb: &str,
    rest: &[String],
) -> Result<Command> {
    let mut params = base_params(kind);
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            flag @ ("--id" | "--hash" | "--category" | "--save-path") => {
                let allowed = match verb {
                    "create-category" => ["--category", "--save-path"].contains(&flag),
                    "edit-category" => ["--category", "--save-path"].contains(&flag),
                    "remove-category" => flag == "--category",
                    "set-category" => ["--id", "--hash", "--category"].contains(&flag),
                    _ => false,
                };
                if !allowed {
                    return Err(anyhow!("{verb} does not accept argument `{flag}`"));
                }
                let key = flag[2..].replace('-', "_");
                insert_once(&mut params, &key, take_value(rest, &mut i, flag)?, flag)?;
            }
            other => return Err(anyhow!("{verb} does not accept argument `{other}`")),
        }
        i += 1;
    }
    match verb {
        "create-category" | "remove-category" if !params.contains_key("category") => {
            return Err(anyhow!("{verb} requires --category"));
        }
        "edit-category"
            if !params.contains_key("category") || !params.contains_key("save_path") =>
        {
            return Err(anyhow!("edit-category requires --category and --save-path"));
        }
        "set-category" => {
            require_selector(&params, verb)?;
            reject_both_selectors(&params, verb)?;
        }
        _ => {}
    }
    Ok(Command::Curated {
        action,
        params: Value::Object(params),
    })
}

fn parse_tags(
    kind: ServiceKind,
    action: &'static str,
    verb: &str,
    rest: &[String],
) -> Result<Command> {
    let mut params = base_params(kind);
    let mut tags = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            flag @ ("--id" | "--hash") => {
                if !matches!(verb, "add-tags" | "remove-tags") {
                    return Err(anyhow!("{verb} does not accept argument `{flag}`"));
                }
                insert_once(
                    &mut params,
                    &flag[2..],
                    take_value(rest, &mut i, flag)?,
                    flag,
                )?;
            }
            flag @ "--tag" => tags.push(take_value(rest, &mut i, flag)?),
            other => return Err(anyhow!("{verb} does not accept argument `{other}`")),
        }
        i += 1;
    }
    if !tags.is_empty() {
        params.insert("tags".into(), json!(tags));
    }
    match verb {
        "create-tags" | "delete-tags" if !params.contains_key("tags") => {
            return Err(anyhow!("{verb} requires at least one --tag"));
        }
        "add-tags" => {
            require_selector(&params, verb)?;
            reject_both_selectors(&params, verb)?;
            if !params.contains_key("tags") {
                return Err(anyhow!("add-tags requires at least one --tag"));
            }
        }
        "remove-tags" => {
            require_selector(&params, verb)?;
            reject_both_selectors(&params, verb)?;
        }
        _ => {}
    }
    Ok(Command::Curated {
        action,
        params: Value::Object(params),
    })
}

fn insert_once(
    params: &mut Map<String, Value>,
    key: &str,
    value: String,
    flag: &str,
) -> Result<()> {
    if params.insert(key.to_owned(), json!(value)).is_some() {
        return Err(anyhow!("received duplicate {flag}"));
    }
    Ok(())
}

fn require_selector(params: &Map<String, Value>, verb: &str) -> Result<()> {
    if !params.contains_key("id") && !params.contains_key("hash") {
        return Err(anyhow!("{verb} requires --id or --hash"));
    }
    Ok(())
}

fn reject_both_selectors(params: &Map<String, Value>, verb: &str) -> Result<()> {
    if params.contains_key("id") && params.contains_key("hash") {
        return Err(anyhow!("{verb} accepts exactly one of --id or --hash"));
    }
    Ok(())
}

/// Initial params map carrying the positional service.
fn base_params(kind: ServiceKind) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("service".into(), json!(kind.as_str()));
    params
}

/// Advance `i` to the value after a flag, rejecting a missing/flag-like value.
fn take_value(rest: &[String], i: &mut usize, flag: &str) -> Result<String> {
    *i += 1;
    rest.get(*i)
        .filter(|v| !v.starts_with("--"))
        .cloned()
        .ok_or_else(|| anyhow!("{flag} requires a value"))
}

/// Resolve a friendly CLI `verb` against [`VERBS`] (the SSOT) to its
/// DownloadClient curated action name.
///
/// Returns `Ok(None)` when `verb` is not a DownloadClient curated verb (the
/// caller falls through), and an `Err` only if the VERBS↔registry wiring is
/// broken — an invariant guarded by `tests/parity.rs`, surfaced here as a clean
/// parse error instead of a panic.
fn resolve(verb: &str) -> Result<Option<&'static str>> {
    let Some((_, action)) = VERBS.iter().find(|(cli_verb, _)| *cli_verb == verb) else {
        return Ok(None);
    };
    curated_command(action)
        .filter(|cmd| cmd.capability == Capability::DownloadClient)
        .map(|cmd| Some(cmd.name))
        .ok_or_else(|| anyhow!("internal: verb `{verb}` has no DownloadClient descriptor"))
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
