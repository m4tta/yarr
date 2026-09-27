use super::*;

#[test]
fn multipart_file_payload_preserves_field_metadata_and_bytes() {
    let field = MultipartField::File {
        name: "archive".into(),
        file_name: "backup.zip".into(),
        media_type: "application/zip".into(),
        bytes: vec![0, 1, 2],
    };

    let MultipartField::File {
        name,
        file_name,
        media_type,
        bytes,
    } = field
    else {
        panic!("expected file field");
    };
    assert_eq!(name, "archive");
    assert_eq!(file_name, "backup.zip");
    assert_eq!(media_type, "application/zip");
    assert_eq!(bytes, [0, 1, 2]);
}

#[tokio::test]
async fn qbittorrent_multipart_rebuilds_once_after_stale_sid() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::post;

    let logins = Arc::new(AtomicUsize::new(0));
    let uploads = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let login_count = Arc::clone(&logins);
    let upload_bodies = Arc::clone(&uploads);
    let app = axum::Router::new()
        .route(
            "/api/v2/auth/login",
            post(move || {
                let login_count = Arc::clone(&login_count);
                async move {
                    let attempt = login_count.fetch_add(1, Ordering::SeqCst);
                    let sid = if attempt == 0 { "stale" } else { "fresh" };
                    ([(header::SET_COOKIE, format!("SID={sid}; path=/"))], "Ok.")
                }
            }),
        )
        .route(
            "/api/v2/torrents/add",
            post(move |headers: HeaderMap, body: Bytes| {
                let upload_bodies = Arc::clone(&upload_bodies);
                async move {
                    upload_bodies.lock().unwrap().push(body.to_vec());
                    let cookie = headers
                        .get(header::COOKIE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default();
                    if cookie.contains("SID=fresh") {
                        (StatusCode::OK, "")
                    } else {
                        (StatusCode::FORBIDDEN, "stale SID")
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let service = ServiceConfig {
        name: "qbit".into(),
        kind: ServiceKind::Qbittorrent,
        base_url: format!("http://{address}"),
        username: Some("user".into()),
        password: Some("password".into()),
        ..ServiceConfig::default()
    };
    let client = YarrClient::new(&crate::config::YarrConfig {
        services: vec![service.clone()],
    })
    .unwrap();
    let url = reqwest::Url::parse(&format!("http://{address}/api/v2/torrents/add")).unwrap();
    let result = client
        .request_openapi_url(OpenApiRequest {
            method: Method::POST,
            service: &service,
            url,
            headers: &[],
            body: Some(EncodedRequestBody::Multipart(vec![
                MultipartField::Text {
                    name: "category".into(),
                    value: "tests".into(),
                },
                MultipartField::File {
                    name: "torrents".into(),
                    file_name: "fixture.torrent".into(),
                    media_type: "application/x-bittorrent".into(),
                    bytes: vec![0, 1, 2, 3],
                },
            ])),
            accept: Some("text/plain"),
            expected_encoding: crate::openapi::BodyEncoding::Text,
            expected_media_type: "text/plain",
        })
        .await
        .unwrap();
    server.abort();

    assert_eq!(result["status"], 200);
    assert_eq!(logins.load(Ordering::SeqCst), 2);
    let bodies = uploads.lock().unwrap();
    assert_eq!(
        bodies.len(),
        2,
        "multipart POST must be retried exactly once"
    );
    for body in bodies.iter() {
        assert!(
            body.windows(b"fixture.torrent".len())
                .any(|part| part == b"fixture.torrent")
        );
        assert!(body.windows(4).any(|part| part == [0, 1, 2, 3]));
    }
}
