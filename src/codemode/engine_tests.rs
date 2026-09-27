//! Engine harness tests — drive `run` directly with a mock tool caller (no tokio,
//! no real services).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use super::{ArtifactWriter, EmbedCaller, EngineLimits, ToolCaller, run};
use crate::codemode::{CODEMODE_MEMORY_LIMIT, CODEMODE_STACK_LIMIT, build_preamble};
use crate::config::ServiceKind;

fn limits(ttl: Duration) -> EngineLimits {
    limits_with_memory(ttl, CODEMODE_MEMORY_LIMIT)
}

fn limits_with_memory(ttl: Duration, memory_bytes: usize) -> EngineLimits {
    EngineLimits {
        memory_bytes,
        stack_bytes: CODEMODE_STACK_LIMIT,
        deadline: Instant::now() + ttl,
    }
}

/// A mock caller that echoes the action id + params back as a JSON object.
fn echo_caller() -> ToolCaller {
    Box::new(|id: &str, params_json: &str| {
        Ok(format!(r#"{{"echo":"{id}","params":{params_json}}}"#))
    })
}

/// A no-op artifact writer for tests that don't exercise `writeArtifact`.
fn no_write() -> ArtifactWriter {
    Box::new(|_path, _content, _opts| Err("artifacts disabled in this test".to_string()))
}

/// A no-op embed bridge for tests that don't exercise semantic search — always
/// returns an empty scores object, exactly what a disabled/unreachable TEI
/// would produce (see `EmbedCaller`'s "never Err" contract).
fn no_embed() -> EmbedCaller {
    Box::new(|_query| Ok("{}".to_string()))
}

#[test]
fn plain_expression_returns_value() {
    let out = run(
        "async () => 6 * 7",
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result, serde_json::json!(42));
}

#[test]
fn calltool_round_trips_through_on_call() {
    let code = r#"
        async () => {
            const a = await callTool("list", { service: "sonarr" });
            const b = await callTool("service_status", { service: "radarr" });
            return { first: a.echo, second: b.echo, viaParams: a.params.service };
        }
    "#;
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result["first"], "list");
    assert_eq!(out.result["second"], "service_status");
    assert_eq!(out.result["viaParams"], "sonarr");
}

#[test]
fn console_output_is_captured() {
    let code = r#"async () => { console.log("hello", 1); console.error("boom"); return "ok"; }"#;
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result, serde_json::json!("ok"));
    assert_eq!(
        out.logs,
        vec!["hello 1".to_string(), "ERROR boom".to_string()]
    );
}

#[test]
fn thrown_tool_error_surfaces_as_err() {
    let failing: ToolCaller = Box::new(|_id, _params| Err("upstream exploded".to_string()));
    let code =
        r#"async () => { await callTool("list", { service: "sonarr" }); return "unreachable"; }"#;
    let err = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        failing,
        no_write(),
        no_embed(),
        None,
    )
    .unwrap_err();
    assert!(err.contains("upstream exploded"), "got: {err}");
}

#[test]
fn script_throw_surfaces_as_err() {
    let code = r#"async () => { throw new Error("nope"); }"#;
    let err = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap_err();
    assert!(err.contains("nope"), "got: {err}");
}

#[test]
fn write_artifact_round_trips_through_on_write() {
    let code = r#"async () => {
        const r = await writeArtifact("out/report.json", JSON.stringify({ ok: true }), { contentType: "application/json" });
        return r;
    }"#;
    // Capturing writer: echoes a receipt for whatever it's handed.
    let writer: ArtifactWriter = Box::new(|path: &str, content: &str, _opts: &str| {
        Ok(format!(r#"{{"path":"{path}","bytes":{}}}"#, content.len()))
    });
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        writer,
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result["path"], "out/report.json");
    assert!(out.result["bytes"].as_i64().unwrap() > 0);
}

#[test]
fn write_artifact_error_surfaces_as_throw() {
    let code = r#"async () => {
        try { await writeArtifact("x.txt", "hi"); return "ran"; }
        catch (e) { return "blocked:" + e.message; }
    }"#;
    let writer: ArtifactWriter = Box::new(|_p, _c, _o| Err("disk full".to_string()));
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        writer,
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result, serde_json::json!("blocked:disk full"));
}

#[test]
fn infinite_loop_is_interrupted_by_deadline() {
    let code = r#"async () => { while (true) {} }"#;
    let err = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_millis(300)),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap_err();
    // Either the timeout guard or the interrupted job surfaces — both are errors.
    assert!(!err.is_empty(), "expected an error for a runaway loop");
}

#[test]
fn memory_limit_is_enforced() {
    // A deliberately small test budget proves `rt.set_memory_limit` is active
    // independently of the production heap size. The deadline is generous so
    // memory — not time — trips it.
    let code = r#"async () => { const s = "x".repeat(100 * 1024 * 1024); return s.length; }"#;
    let err = run(
        code,
        &build_preamble(&[]),
        &limits_with_memory(Duration::from_secs(10), 64 * 1024 * 1024),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap_err();
    assert!(
        !err.is_empty(),
        "expected an error for an over-budget allocation"
    );
    assert!(
        !err.contains("timed out"),
        "expected a memory error, not a timeout: {err}"
    );
}

#[test]
fn large_structured_tool_result_can_be_summarized_within_current_heap() {
    const ROW_COUNT: usize = 2_000;
    const ROW_BYTES: usize = 6_000;

    // Model a large movie library with many scalar and nested fields. Keeping
    // the payload structured matters: one giant string does not reproduce the
    // QuickJS object-graph overhead of a real generated API response.
    let mut row = serde_json::json!({
        "id": 1,
        "title": "Synthetic Movie",
        "originalTitle": "Synthetic Original Title",
        "sortTitle": "synthetic movie",
        "status": "released",
        "overview": "synthetic overview ".repeat(40),
        "inCinemas": "2026-01-01T00:00:00Z",
        "digitalRelease": "2026-02-01T00:00:00Z",
        "physicalRelease": "2026-03-01T00:00:00Z",
        "year": 2026,
        "runtime": 123,
        "monitored": true,
        "hasFile": true,
        "qualityProfileId": 1,
        "path": "/synthetic/library/movie",
        "rootFolderPath": "/synthetic/library",
        "genres": ["Adventure", "Comedy", "Drama", "Science Fiction"],
        "tags": [1, 2, 3, 4],
        "ratings": {
            "imdb": {"votes": 12345, "value": 7.5, "type": "user"},
            "tmdb": {"votes": 6789, "value": 7.2, "type": "user"},
            "rottenTomatoes": {"votes": 456, "value": 82, "type": "critic"}
        },
        "alternateTitles": (0..12).map(|n| serde_json::json!({
            "sourceType": 0, "movieMetadataId": 1, "title": format!("Synthetic Alternate {n}")
        })).collect::<Vec<_>>(),
        "images": (0..10).map(|n| serde_json::json!({
            "coverType": "poster",
            "url": format!("/synthetic/image/{n}.jpg"),
            "remoteUrl": format!("https://invalid.example/image/{n}.jpg")
        })).collect::<Vec<_>>(),
        "collection": {
            "title": "Synthetic Collection",
            "tmdbId": 999,
            "images": [{"coverType": "poster", "url": "/synthetic/collection.jpg"}]
        },
        "movieFile": {
            "id": 1,
            "relativePath": "Synthetic.Movie.mkv",
            "path": "/synthetic/library/movie/Synthetic.Movie.mkv",
            "size": 1234567890_u64,
            "dateAdded": "2026-01-01T00:00:00Z",
            "quality": {"quality": {"id": 7, "name": "Synthetic", "source": "web"}, "revision": {"version": 1}},
            "languages": [{"id": 1, "name": "English"}],
            "mediaInfo": {
                "audioBitrate": 384000,
                "audioChannels": 6.0,
                "audioCodec": "Synthetic Audio",
                "audioLanguages": "eng",
                "audioStreamCount": 1,
                "videoBitDepth": 10,
                "videoBitrate": 8000000,
                "videoCodec": "Synthetic Video",
                "videoDynamicRange": "Synthetic Range",
                "videoFps": 24.0,
                "resolution": "1920x1080",
                "runTime": "02:03:00",
                "scanType": "Progressive",
                "subtitles": "eng",
                "videoStreamCount": 1
            }
        },
        "syntheticPadding": ""
    });
    let base_row = serde_json::to_string(&row).unwrap();
    assert!(base_row.len() < ROW_BYTES);
    row["syntheticPadding"] = serde_json::Value::String("x".repeat(ROW_BYTES - base_row.len()));
    let row = serde_json::to_string(&row).unwrap();
    assert_eq!(row.len(), ROW_BYTES);

    let mut payload = String::with_capacity(2 + ROW_COUNT * (ROW_BYTES + 1));
    payload.push('[');
    for index in 0..ROW_COUNT {
        if index != 0 {
            payload.push(',');
        }
        payload.push_str(&row);
    }
    payload.push(']');
    assert!((12_000_000..12_500_000).contains(&payload.len()));
    let payload: Arc<str> = payload.into();
    let caller = |payload: Arc<str>| -> ToolCaller {
        Box::new(move |_id, _params| Ok(payload.as_ref().to_owned()))
    };
    let code = r#"async () => (await callTool("large_library", {})).length"#;
    let services = ServiceKind::ALL
        .into_iter()
        .map(|kind| (kind.as_str().to_owned(), kind))
        .collect::<Vec<_>>();
    let preamble = build_preamble(&services);

    let out = run(
        code,
        &preamble,
        &limits(Duration::from_secs(10)),
        caller(payload),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result, serde_json::json!(ROW_COUNT));
}

#[test]
fn stalled_calltool_is_bounded_by_deadline() {
    // The deadline interrupt handler can't preempt a blocking NATIVE call, so a
    // `callTool` that sleeps past the deadline runs to completion. But once it
    // returns, `drain_jobs` checks the deadline BETWEEN microtask jobs and aborts
    // — so the wall-clock deadline still terminates the script. (A genuinely hung
    // call is otherwise bounded by the upstream HTTP client's own timeout, not the
    // codemode deadline.)
    let slow: ToolCaller = Box::new(|_id, _params| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        Ok("null".to_string())
    });
    let code = r#"async () => { await callTool("slow", {}); return "done"; }"#;
    let err = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_millis(100)),
        slow,
        no_write(),
        no_embed(),
        None,
    )
    .unwrap_err();
    assert!(err.contains("timed out"), "got: {err}");
}

#[test]
fn search_with_no_lexical_overlap_and_no_semantic_score_finds_nothing() {
    // Baseline/control for the next test: a query sharing zero tokens with any
    // catalog entry, with the embed bridge always returning "no scores" (as it
    // does when semantic search is disabled), finds nothing — today's
    // lexical-only behavior, unchanged.
    let code = r#"async () => (await codemode.search("xyzzy plugh nonsense")).results"#;
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        no_embed(),
        None,
    )
    .unwrap();
    assert_eq!(out.result, serde_json::json!([]));
}

#[test]
fn search_blends_in_a_high_semantic_score_for_zero_lexical_overlap_query() {
    // Same nonsense query as above, but now the embed bridge (standing in for a
    // real TEI call) reports a strong semantic match for one specific catalog
    // path. That path must now appear in results — proving the JS
    // callTool-adjacent __yarrEmbedQuery bridge, its JSON round-trip, and the
    // blend-into-score logic in codemode.search all work end to end, not just
    // that each piece compiles in isolation.
    let embed: EmbedCaller = Box::new(|_query| Ok(r#"{"api.<service>.get":0.9}"#.to_string()));
    let code = r#"async () => (await codemode.search("xyzzy plugh nonsense")).results"#;
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        embed,
        None,
    )
    .unwrap();
    let paths: Vec<&str> = out
        .result
        .as_array()
        .expect("results should be an array")
        .iter()
        .map(|r| r["path"].as_str().expect("path should be a string"))
        .collect();
    assert_eq!(
        paths,
        vec!["api.<service>.get"],
        "the semantically-scored path should be the only result"
    );
}

#[test]
fn search_embed_bridge_is_not_called_for_an_empty_query() {
    // codemode.search("") lists everything unranked — there's nothing to embed
    // and rank against, so the JS should short-circuit before ever calling
    // __yarrEmbedQuery. Tracked via a shared flag rather than a panicking
    // bridge: panicking across the rquickjs FFI boundary has unclear
    // unwind-safety, so a flag the assertion checks afterward is the safe way
    // to observe "was this called" without risking the test process itself.
    let was_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let was_called_in_closure = was_called.clone();
    let embed: EmbedCaller = Box::new(move |_query| {
        was_called_in_closure.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok("{}".to_string())
    });
    let code = r#"async () => (await codemode.search("")).total"#;
    let out = run(
        code,
        &build_preamble(&[]),
        &limits(Duration::from_secs(5)),
        echo_caller(),
        no_write(),
        embed,
        None,
    )
    .unwrap();
    // Zero configured services -> only the 4 generic api.<service>.* entries.
    assert_eq!(out.result, serde_json::json!(4));
    assert!(
        !was_called.load(std::sync::atomic::Ordering::SeqCst),
        "codemode.search(\"\") must not call the embed bridge"
    );
}
