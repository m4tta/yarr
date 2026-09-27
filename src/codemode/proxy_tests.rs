//! Preamble generation tests.

use super::*;
use crate::config::ServiceKind;

fn services() -> Vec<(String, ServiceKind)> {
    vec![
        ("sonarr".to_string(), ServiceKind::Sonarr),
        ("radarr".to_string(), ServiceKind::Radarr),
        ("plex".to_string(), ServiceKind::Plex),
    ]
}

#[test]
fn preamble_defines_calltool_and_runner() {
    let pre = build_preamble(&[]);
    assert!(pre.contains("globalThis.callTool ="));
    assert!(pre.contains("__yarrEmitToolCall"));
    assert!(pre.contains("globalThis.__yarrRun ="));
    assert!(pre.contains("globalThis.console ="));
}

#[test]
fn per_service_namespaces_bake_in_the_service() {
    let pre = build_preamble(&services());
    // One object per configured service, keyed by service name.
    assert!(pre.contains(r#"globalThis["sonarr"] = {"#));
    assert!(pre.contains(r#"globalThis["radarr"] = {"#));
    assert!(pre.contains(r#"globalThis["plex"] = {"#));
    // Spec-backed kinds dispatch each generated operation through the `op` action,
    // with the service + op baked in (never passed by the script).
    assert!(pre.contains(r#"["service_status"]: (params) => callTool("service_status""#));
    assert!(pre.contains(r#"["get_series"]: (params) => callTool("op""#));
    assert!(pre.contains(r#"op: "get_series""#));
    assert!(pre.contains(r#"service: "sonarr""#));
    assert!(pre.contains(r#"service: "radarr""#));
}

#[test]
fn qbittorrent_curated_helpers_are_additive_and_sab_does_not_advertise_them() {
    let pre = build_preamble(&[
        ("qbit_movies".to_string(), ServiceKind::Qbittorrent),
        ("sabnzbd".to_string(), ServiceKind::Sabnzbd),
    ]);
    assert!(pre.contains(r#"globalThis["qbit_movies"] = {"#));
    assert!(pre.contains(r#"["download_set_limits"]: (params) => callTool("download_set_limits""#));

    let sab_start = pre.find(r#"globalThis["sabnzbd"] = {"#).unwrap();
    let sab_end = pre[sab_start..].find("};\n").unwrap() + sab_start;
    assert!(!pre[sab_start..sab_end].contains("download_set_limits"));
}

#[test]
fn qbittorrent_multipart_controls_are_injected_for_describe_only() {
    let pre = build_preamble(&[("qbit".to_string(), ServiceKind::Qbittorrent)]);
    assert!(pre.contains(r#""requiredAlternatives"#));
    assert!(pre.contains(r#""multipartFileBase64"#));
    assert!(pre.contains("call.request_body"));
    // Search still projects a fixed compact result instead of returning schemas.
    assert!(pre.contains("return { e: { path: e.path, service: e.service"));
}

#[test]
fn no_flat_tools_namespace() {
    // The old flat `tools.<action>({service})` surface (the service-param leak) is
    // gone — everything is reached through a per-service callable.
    let pre = build_preamble(&services());
    assert!(!pre.contains("globalThis.tools"));
    assert!(!pre.contains(r#"tools["list"]"#));
}

#[test]
fn reserved_global_name_is_not_clobbered() {
    // A service literally named `api` must not get a top-level binding that would
    // overwrite the raw-API client; the client itself is still present.
    let pre = build_preamble(&[("api".to_string(), ServiceKind::Sonarr)]);
    assert!(!pre.contains(r#"globalThis["api"] = {"#));
    assert!(pre.contains("globalThis.api = {};"));
}

#[test]
fn api_namespace_generated_per_configured_service() {
    let pre = build_preamble(&services());
    assert!(pre.contains("globalThis.api = {};"));
    assert!(pre.contains(r#"globalThis.api["sonarr"]"#));
    assert!(pre.contains(r#"globalThis.api["radarr"]"#));
    // get/post/put/delete sugar over the api_* passthrough actions.
    assert!(pre.contains(r#"callTool("api_get", { service: "sonarr""#));
    assert!(pre.contains(r#"callTool("api_delete", { service: "radarr""#));
}

#[test]
fn api_namespace_empty_when_no_services() {
    let pre = build_preamble(&[]);
    assert!(pre.contains("globalThis.api = {};"));
    assert!(!pre.contains("globalThis.api[\""));
}

#[test]
fn preamble_injects_discovery_catalog_and_helpers() {
    let pre = build_preamble(&services());
    assert!(pre.contains("globalThis.__codemodeCatalog = ["));
    assert!(pre.contains("globalThis.codemode.search ="));
    assert!(pre.contains("globalThis.codemode.describe ="));
    // The catalog embeds fully-qualified generated callable paths + a destructive
    // flag (DELETE ops).
    assert!(pre.contains(r#""path":"sonarr.get_series""#));
    assert!(pre.contains("\"destructive\":true"));
    // The type catalog is injected so describe/search can surface response types.
    assert!(pre.contains("globalThis.__codemodeTypes = ["));
    assert!(pre.contains("sonarr.SeriesResource"));
}

#[test]
fn snippet_verbs_are_not_callable_namespaces() {
    let pre = build_preamble(&services());
    // Snippet store verbs are reachable only via codemode.run/snippets — never as
    // per-service callables (no incidental file writes/deletes).
    assert!(!pre.contains(r#"["snippet_save"]:"#));
    assert!(!pre.contains(r#"["snippet_run"]:"#));
    // But the explicit discovery/run helpers ARE present.
    assert!(pre.contains("globalThis.codemode.run ="));
    assert!(pre.contains("globalThis.codemode.snippets ="));
    // And `input` is wired (defaults to null for non-snippet runs).
    assert!(pre.contains("globalThis.input ="));
}
