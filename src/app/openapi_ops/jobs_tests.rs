use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, Response, StatusCode};
use serde_json::{Value, json};

use crate::app::YarrService;
use crate::config::{ServiceConfig, ServiceKind, YarrConfig};
use crate::yarr::YarrClient;

enum Behavior {
    Sequence(Mutex<VecDeque<&'static str>>),
    PollResponse(Value),
    PostError,
    PollError,
    SlowPost,
    SlowPoll,
}

#[derive(Clone)]
struct ServerState {
    behavior: Arc<Behavior>,
    requests: Arc<AtomicUsize>,
    posts: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
}

struct Harness {
    service: YarrService,
    state: ServerState,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn harness(kind: ServiceKind, behavior: Behavior) -> Harness {
    let state = ServerState {
        behavior: Arc::new(behavior),
        requests: Arc::new(AtomicUsize::new(0)),
        posts: Arc::new(AtomicUsize::new(0)),
        polls: Arc::new(AtomicUsize::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .fallback(axum::routing::any(command_endpoint))
        .with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = YarrConfig {
        services: vec![ServiceConfig {
            name: kind.as_str().to_owned(),
            kind,
            base_url: format!("http://{address}"),
            api_key: Some("secret".into()),
            ..ServiceConfig::default()
        }],
    };
    let client = YarrClient::new(&config).unwrap();
    Harness {
        service: YarrService::new(client, config),
        state,
        server,
    }
}

async fn command_endpoint(
    State(state): State<ServerState>,
    request: Request<Body>,
) -> Response<Body> {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let method = request.method();
    let path = request.uri().path();
    if method == Method::POST && path == "/api/v3/command" {
        state.posts.fetch_add(1, Ordering::SeqCst);
        return match state.behavior.as_ref() {
            Behavior::PostError => {
                json_response(StatusCode::SERVICE_UNAVAILABLE, json!({"error": "private"}))
            }
            Behavior::SlowPost => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                json_response(StatusCode::CREATED, json!({"id": 41, "status": "queued"}))
            }
            _ => json_response(StatusCode::CREATED, json!({"id": 41, "status": "queued"})),
        };
    }
    if method == Method::GET && path == "/api/v3/command/41" {
        state.polls.fetch_add(1, Ordering::SeqCst);
        return match state.behavior.as_ref() {
            Behavior::Sequence(statuses) => {
                let status = statuses.lock().unwrap().pop_front().unwrap_or("started");
                json_response(StatusCode::OK, json!({"id": 41, "status": status}))
            }
            Behavior::PollResponse(value) => json_response(StatusCode::OK, value.clone()),
            Behavior::PollError => {
                json_response(StatusCode::SERVICE_UNAVAILABLE, json!({"error": "private"}))
            }
            Behavior::SlowPoll => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                json_response(StatusCode::OK, json!({"id": 41, "status": "started"}))
            }
            Behavior::PostError | Behavior::SlowPost => {
                json_response(StatusCode::OK, json!({"id": 41, "status": "started"}))
            }
        };
    }
    json_response(StatusCode::NOT_FOUND, json!({"error": "not found"}))
}

fn json_response(status: StatusCode, value: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn command_args(kind: ServiceKind, wait: Option<Value>) -> Value {
    let body = match kind {
        ServiceKind::Sonarr => json!({"name": "RescanSeries", "seriesId": 7}),
        ServiceKind::Radarr => json!({"name": "RescanMovie", "movieId": 7}),
        _ => unreachable!(),
    };
    let mut args = json!({"body": body});
    if let Some(wait) = wait {
        args["waitForCompletion"] = wait;
    }
    args
}

#[tokio::test]
async fn wait_tracks_sonarr_and_radarr_to_completion_with_one_post() {
    for kind in [ServiceKind::Sonarr, ServiceKind::Radarr] {
        let harness = harness(
            kind,
            Behavior::Sequence(Mutex::new(VecDeque::from(["started", "completed"]))),
        )
        .await;
        let result = harness
            .service
            .execute_operation(
                kind.as_str(),
                "post_command",
                &command_args(
                    kind,
                    Some(json!({"timeoutSeconds": 2, "pollIntervalMs": 100})),
                ),
            )
            .await
            .unwrap();

        assert_eq!(result["commandId"], 41);
        assert_eq!(result["status"], "completed");
        assert_eq!(result["finished"], true);
        assert_eq!(result["command"]["status"], "completed");
        assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
        assert_eq!(harness.state.polls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn wait_returns_terminal_failure_as_a_typed_outcome() {
    let harness = harness(
        ServiceKind::Sonarr,
        Behavior::Sequence(Mutex::new(VecDeque::from(["failed"]))),
    )
    .await;
    let result = harness
        .service
        .execute_operation(
            "sonarr",
            "post_command",
            &command_args(
                ServiceKind::Sonarr,
                Some(json!({"timeoutSeconds": 2, "pollIntervalMs": 100})),
            ),
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "failed");
    assert_eq!(result["finished"], true);
    assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.polls.load(Ordering::SeqCst), 1);
}

#[test]
fn terminal_status_distinguishes_all_supported_outcomes() {
    for (upstream, outcome) in [
        ("completed", "completed"),
        ("failed", "failed"),
        ("orphaned", "failed"),
        ("aborted", "aborted"),
        ("cancelled", "cancelled"),
    ] {
        assert_eq!(
            super::terminal_status(&json!({"status": upstream})),
            Some(outcome)
        );
    }
    assert_eq!(super::terminal_status(&json!({"status": "started"})), None);
}

#[tokio::test]
async fn absolute_deadline_includes_an_in_flight_poll() {
    let harness = harness(ServiceKind::Sonarr, Behavior::SlowPoll).await;
    let result = harness
        .service
        .execute_operation(
            "sonarr",
            "post_command",
            &command_args(
                ServiceKind::Sonarr,
                Some(json!({"timeoutSeconds": 1, "pollIntervalMs": 100})),
            ),
        )
        .await
        .unwrap();

    assert_eq!(result["commandId"], 41);
    assert_eq!(result["status"], "timed_out");
    assert_eq!(result["finished"], false);
    assert!(result["note"].as_str().unwrap().contains("resume"));
    assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.polls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn absolute_deadline_includes_the_initial_post_without_retrying_it() {
    let harness = harness(ServiceKind::Sonarr, Behavior::SlowPost).await;
    let error = harness
        .service
        .execute_operation(
            "sonarr",
            "post_command",
            &command_args(
                ServiceKind::Sonarr,
                Some(json!({"timeoutSeconds": 1, "pollIntervalMs": 100})),
            ),
        )
        .await
        .expect_err("a slow submission must hit the absolute deadline");

    assert!(
        error.to_string().contains("submission outcome is unknown"),
        "{error:#}"
    );
    assert_eq!(harness.state.requests.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn initial_post_http_failure_is_not_retried() {
    let harness = harness(ServiceKind::Sonarr, Behavior::PostError).await;
    harness
        .service
        .execute_operation(
            "sonarr",
            "post_command",
            &command_args(
                ServiceKind::Sonarr,
                Some(json!({"timeoutSeconds": 2, "pollIntervalMs": 100})),
            ),
        )
        .await
        .expect_err("an upstream submission failure must be returned");

    assert_eq!(harness.state.requests.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn poll_error_preserves_command_id_without_retrying_the_post() {
    let harness = harness(ServiceKind::Sonarr, Behavior::PollError).await;
    let result = harness
        .service
        .execute_operation(
            "sonarr",
            "post_command",
            &command_args(
                ServiceKind::Sonarr,
                Some(json!({"timeoutSeconds": 2, "pollIntervalMs": 100})),
            ),
        )
        .await
        .unwrap();

    assert_eq!(result["commandId"], 41);
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["finished"], false);
    assert_eq!(result["command"]["status"], "queued");
    assert!(result["note"].as_str().unwrap().contains("resume"));
    assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.polls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn malformed_poll_response_is_unknown_and_preserves_submitted_id() {
    for malformed in [
        json!({"id": 42, "status": "completed"}),
        json!({"id": 41}),
        json!({"id": 41, "status": "new-upstream-state"}),
    ] {
        let harness = harness(ServiceKind::Sonarr, Behavior::PollResponse(malformed)).await;
        let result = harness
            .service
            .execute_operation(
                "sonarr",
                "post_command",
                &command_args(
                    ServiceKind::Sonarr,
                    Some(json!({"timeoutSeconds": 2, "pollIntervalMs": 100})),
                ),
            )
            .await
            .unwrap();

        assert_eq!(result["commandId"], 41);
        assert_eq!(result["status"], "unknown");
        assert_eq!(result["finished"], false);
        assert!(result["note"].as_str().unwrap().contains("resume"));
        assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
        assert_eq!(harness.state.polls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn invalid_wait_controls_make_no_upstream_request() {
    let sonarr = harness(
        ServiceKind::Sonarr,
        Behavior::Sequence(Mutex::new(VecDeque::new())),
    )
    .await;
    let cases = [
        ("get_series", json!({"waitForCompletion": {}})),
        (
            "post_command",
            command_args(ServiceKind::Sonarr, Some(Value::Null)),
        ),
        (
            "post_command",
            command_args(ServiceKind::Sonarr, Some(json!({"timeoutSeconds": 0}))),
        ),
        (
            "post_command",
            command_args(ServiceKind::Sonarr, Some(json!({"timeoutSeconds": 21}))),
        ),
        (
            "post_command",
            command_args(ServiceKind::Sonarr, Some(json!({"pollIntervalMs": 99}))),
        ),
        (
            "post_command",
            command_args(ServiceKind::Sonarr, Some(json!({"pollIntervalMs": 5001}))),
        ),
        (
            "post_command",
            command_args(ServiceKind::Sonarr, Some(json!({"surprise": true}))),
        ),
    ];

    for (operation, args) in cases {
        sonarr
            .service
            .execute_operation("sonarr", operation, &args)
            .await
            .expect_err("invalid wait controls must fail");
    }
    assert_eq!(sonarr.state.requests.load(Ordering::SeqCst), 0);
    assert_eq!(sonarr.state.posts.load(Ordering::SeqCst), 0);
    assert_eq!(sonarr.state.polls.load(Ordering::SeqCst), 0);

    let unsupported = harness(
        ServiceKind::Prowlarr,
        Behavior::Sequence(Mutex::new(VecDeque::new())),
    )
    .await;
    unsupported
        .service
        .execute_operation(
            "prowlarr",
            "post_command",
            &json!({
                "body": {"name": "CheckHealth"},
                "waitForCompletion": {},
            }),
        )
        .await
        .expect_err("Prowlarr command waits are unsupported");
    assert_eq!(unsupported.state.requests.load(Ordering::SeqCst), 0);
    assert_eq!(unsupported.state.posts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn absent_wait_control_keeps_the_raw_post_response() {
    let harness = harness(
        ServiceKind::Sonarr,
        Behavior::Sequence(Mutex::new(VecDeque::new())),
    )
    .await;
    let result = harness
        .service
        .execute_operation(
            "sonarr",
            "post_command",
            &command_args(ServiceKind::Sonarr, None),
        )
        .await
        .unwrap();

    assert_eq!(result, json!({"id": 41, "status": "queued"}));
    assert_eq!(harness.state.posts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.state.polls.load(Ordering::SeqCst), 0);
}
