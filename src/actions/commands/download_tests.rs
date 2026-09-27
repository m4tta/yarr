use super::DOWNLOAD_COMMANDS;
use crate::actions::model::{READ_SCOPE, WRITE_SCOPE};
use crate::capability::Capability;

/// READ verbs.
const READ_COMMANDS: &[&str] = &[
    "download_queue",
    "download_transfer",
    "download_categories",
    "download_tags",
];
/// WRITE verbs (mutating; `download_remove` is additionally destructive).
const WRITE_COMMANDS: &[&str] = &[
    "download_add",
    "download_pause",
    "download_resume",
    "download_remove",
    "download_set_limits",
    "download_create_category",
    "download_edit_category",
    "download_remove_category",
    "download_set_category",
    "download_create_tags",
    "download_delete_tags",
    "download_add_tags",
    "download_remove_tags",
];

#[test]
fn registers_all_download_commands() {
    let names: Vec<&str> = DOWNLOAD_COMMANDS.iter().map(|c| c.name).collect();
    for expected in READ_COMMANDS.iter().chain(WRITE_COMMANDS) {
        assert!(
            names.contains(expected),
            "missing download command {expected}"
        );
    }
    assert_eq!(
        DOWNLOAD_COMMANDS.len(),
        READ_COMMANDS.len() + WRITE_COMMANDS.len()
    );
}

#[test]
fn all_commands_are_download_capability() {
    for cmd in DOWNLOAD_COMMANDS {
        assert_eq!(
            cmd.capability,
            Capability::DownloadClient,
            "{} must be DownloadClient-scoped",
            cmd.name
        );
    }
}

#[test]
fn action_names_are_capability_prefixed_for_global_uniqueness() {
    // Registry action names are globally unique; ArrManager already owns `queue`
    // (C1). Download verbs must use the `download_` prefix so they cannot collide.
    for cmd in DOWNLOAD_COMMANDS {
        assert!(
            cmd.name.starts_with("download_"),
            "{} must be download_-prefixed for global action-name uniqueness",
            cmd.name
        );
    }
}

#[test]
fn queue_is_read_scope_and_non_mutating() {
    for cmd in DOWNLOAD_COMMANDS
        .iter()
        .filter(|c| READ_COMMANDS.contains(&c.name))
    {
        assert_eq!(cmd.required_scope, READ_SCOPE, "{}", cmd.name);
        assert!(!cmd.mutates, "{} must not mutate", cmd.name);
        assert!(!cmd.destructive, "{} must not be destructive", cmd.name);
    }
}

#[test]
fn write_commands_are_write_scope_mutate_and_only_remove_is_destructive() {
    // All writes use WRITE scope and mutate; only `download_remove` is
    // DESTRUCTIVE (MCP elicits confirmation for it). All of them run
    // immediately.
    for cmd in DOWNLOAD_COMMANDS
        .iter()
        .filter(|c| WRITE_COMMANDS.contains(&c.name))
    {
        assert_eq!(cmd.required_scope, WRITE_SCOPE, "{} scope", cmd.name);
        assert!(cmd.mutates, "{} must mutate", cmd.name);
        let destructive = cmd.name == "download_remove";
        assert_eq!(
            cmd.destructive, destructive,
            "{} destructive must equal destructive={destructive}",
            cmd.name
        );
    }
}

#[test]
fn add_and_remove_declare_required_params() {
    let add = DOWNLOAD_COMMANDS
        .iter()
        .find(|c| c.name == "download_add")
        .unwrap();
    assert!(add.required_params.contains(&"service"));
    assert!(add.required_params.contains(&"url"));

    let remove = DOWNLOAD_COMMANDS
        .iter()
        .find(|c| c.name == "download_remove")
        .unwrap();
    // `id` is OPTIONAL: a remove may target either --id or --hash, so only
    // `service` is required. The handler errors at runtime when neither is given.
    assert!(remove.required_params.contains(&"service"));
    assert!(!remove.required_params.contains(&"id"));
    assert!(remove.optional_params.contains(&"id"));
    assert!(remove.optional_params.contains(&"hash"));
    assert!(remove.optional_params.contains(&"delete_files"));
}
