//! End-to-end tests of the HTTP API against an in-memory database and a mock
//! DJI keychain endpoint.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

use dji_log_viewer::config::{ApiKey, Config};
use dji_log_viewer::synthetic::{SynthFlight, SynthKeys};
use dji_log_viewer::{AppState, app, db};

const GOOD_KEY: &str = "test-key-123";

fn config(api_key: Option<&str>, endpoint: &str) -> Config {
    Config {
        api_key: api_key.and_then(ApiKey::new),
        data_dir: "/nonexistent".into(),
        static_dir: "/nonexistent".into(),
        port: 0,
        max_upload_bytes: 5 * 1024 * 1024,
        keychain_endpoint: endpoint.into(),
        map_tile_url: "https://tiles.example/{z}/{x}/{y}.png".into(),
        map_attribution: "test".into(),
        terrain_url: None,
        terrain_encoding: "terrarium".into(),
        terrain_attribution: "test".into(),
    }
}

async fn app_with(api_key: Option<&str>, endpoint: &str) -> Router {
    let pool = db::connect_memory().await.unwrap();
    app(AppState::new(pool, config(api_key, endpoint)))
}

struct MockDji {
    url: String,
    calls: Arc<AtomicUsize>,
}

/// Mock of the DJI keychain endpoint: 403 for a wrong `Api-Key`, otherwise
/// the keychains for `keys`.
async fn mock_dji(keys: SynthKeys) -> MockDji {
    #[derive(Clone)]
    struct S {
        keys: SynthKeys,
        calls: Arc<AtomicUsize>,
    }
    async fn handler(
        State(s): State<S>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        s.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            body.get("keychainsArray").is_some(),
            "unexpected request body {body}"
        );
        if headers.get("Api-Key").and_then(|v| v.to_str().ok()) != Some(GOOD_KEY) {
            return (StatusCode::FORBIDDEN, Json(json!({"message": "Forbidden"})));
        }
        (
            StatusCode::OK,
            Json(json!({"result": {"code": 0, "msg": "success"}, "data": s.keys.api_keychains()})),
        )
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route("/keychains", post(handler))
        .with_state(S {
            keys,
            calls: calls.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    MockDji {
        url: format!("http://{addr}/keychains"),
        calls,
    }
}

fn multipart(file_name: &str, bytes: &[u8]) -> Request<Body> {
    let boundary = "----testboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Request::post("/api/flights")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Vec<u8>, HeaderMap) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body, headers)
}

async fn send_json(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let (status, body, _) = send(app, req).await;
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, value)
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn upload_list_view_export_and_delete() {
    let app = app_with(None, "http://127.0.0.1:9/unused").await;
    let log = SynthFlight::demo().to_bytes();

    let (status, body) = send_json(
        &app,
        multipart("DJIFlightRecord_2024-06-01_[10-00-00].txt", &log),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["created"], true);
    let id = body["flight"]["id"].as_i64().unwrap();
    assert_eq!(body["flight"]["aircraftName"], "Test Mavic");
    assert_eq!(body["flight"]["productType"], "MavicPro");
    assert_eq!(body["flight"]["logVersion"], 6);
    assert_eq!(body["flight"]["distanceM"], 240.0);
    assert_eq!(body["flight"]["startTime"], "2024-06-01T10:00:00Z");

    // Same file again: de-duplicated by content hash.
    let (status, body) = send_json(&app, multipart("copy.txt", &log)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"], false);
    assert_eq!(body["flight"]["id"].as_i64().unwrap(), id);

    let (status, list) = send_json(&app, get("/api/flights")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!((list[0]["durationS"].as_f64().unwrap() - 60.0).abs() < 0.01);
    assert_eq!(list[0]["maxHeightM"], 40.0);

    let (status, detail) = send_json(&app, get(&format!("/api/flights/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let events = detail["events"].as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e["message"] == "Low battery" && e["level"] == "warning")
    );
    assert!(events.iter().any(|e| e["message"] == "Take-off"));

    let (status, tel) = send_json(&app, get(&format!("/api/flights/{id}/telemetry"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tel["t"].as_array().unwrap().len(), 600);
    assert_eq!(tel["heightM"][200], 40.0);
    assert_eq!(tel["batteryPct"][200], 85.0);
    assert!(tel["rcUplinkPct"][200].is_number());

    let (status, csv, headers) = send(&app, get(&format!("/api/flights/{id}/export/csv"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(csv).unwrap().lines().count(), 601);
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("flight-1-2024-06-01.csv")
    );

    let (status, gpx, _) = send(&app, get(&format!("/api/flights/{id}/export/gpx"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        String::from_utf8(gpx).unwrap().matches("<trkpt").count(),
        600
    );

    let (status, kml, headers) = send(&app, get(&format!("/api/flights/{id}/export/kml"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(kml).unwrap().contains("<LineString>"));
    assert_eq!(
        headers["content-type"],
        "application/vnd.google-earth.kml+xml"
    );

    let (status, body) = send_json(&app, get(&format!("/api/flights/{id}/export/shp"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "bad_request");

    let (status, _) = send_json(
        &app,
        Request::delete(format!("/api/flights/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = send_json(&app, get(&format!("/api/flights/{id}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn rejects_files_that_are_not_dji_logs() {
    let app = app_with(None, "http://127.0.0.1:9/unused").await;

    let (status, body) = send_json(&app, multipart("notes.txt", b"hello world")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let code = body["error"]["code"].as_str().unwrap();
    assert!(
        code == "invalid_log" || code == "unsupported_version",
        "{body}"
    );

    // A prefix claiming an unknown format version.
    let mut fake = vec![0u8; 600];
    fake[0] = 100;
    fake[10] = 42;
    let (status, body) = send_json(&app, multipart("v42.txt", &fake)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "unsupported_version");
    assert!(body["error"]["message"].as_str().unwrap().contains("42"));

    // A truncated real log must not crash the server.
    let log = SynthFlight::demo().to_encrypted_bytes(&SynthKeys::new(1));
    let mock = mock_dji(SynthKeys::new(1)).await;
    let app = app_with(Some(GOOD_KEY), &mock.url).await;
    let (status, body) = send_json(&app, multipart("cut.txt", &log[..log.len() - 7])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let (status, body) = send_json(&app, multipart("empty.txt", b"")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "bad_request");

    let (status, list) = send_json(&app, get("/api/flights")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn encrypted_log_without_api_key() {
    let app = app_with(None, "http://127.0.0.1:9/unused").await;
    let log = SynthFlight::demo().to_encrypted_bytes(&SynthKeys::new(1));
    let (status, body) = send_json(&app, multipart("enc.txt", &log)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "missing_api_key");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("DJI_API_KEY")
    );
}

#[tokio::test]
async fn encrypted_log_with_invalid_api_key() {
    let mock = mock_dji(SynthKeys::new(1)).await;
    let app = app_with(Some("wrong-key-456"), &mock.url).await;
    let log = SynthFlight::demo().to_encrypted_bytes(&SynthKeys::new(1));
    let (status, body) = send_json(&app, multipart("enc.txt", &log)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_api_key");
    // The key must never be echoed back.
    assert!(!body.to_string().contains("wrong-key-456"));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn encrypted_log_is_decrypted_and_keys_are_cached() {
    let keys = SynthKeys::new(7);
    let mock = mock_dji(keys.clone()).await;
    let app = app_with(Some(GOOD_KEY), &mock.url).await;
    let log = SynthFlight::demo().to_encrypted_bytes(&keys);

    let (status, body) = send_json(&app, multipart("enc.txt", &log)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["flight"]["logVersion"], 14);
    assert_eq!(body["flight"]["encrypted"], true);
    let id = body["flight"]["id"].as_i64().unwrap();
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);

    let (_, tel) = send_json(&app, get(&format!("/api/flights/{id}/telemetry"))).await;
    assert_eq!(tel["heightM"][200], 40.0);
    assert!((tel["lat"][200].as_f64().unwrap() - 52.2297).abs() < 1e-6);

    // Delete and re-import: keys come from the SQLite cache, not from DJI.
    let req = Request::delete(format!("/api/flights/{id}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::NO_CONTENT);
    let (status, _) = send_json(&app, multipart("enc.txt", &log)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        1,
        "keychains must be cached"
    );
}

#[tokio::test]
async fn keys_that_do_not_decrypt_are_reported_and_not_cached() {
    // DJI answers, but with keys that do not match the log.
    let mock = mock_dji(SynthKeys::new(2)).await;
    let app = app_with(Some(GOOD_KEY), &mock.url).await;
    let log = SynthFlight::demo().to_encrypted_bytes(&SynthKeys::new(1));

    for attempt in 1..=2 {
        let (status, body) = send_json(&app, multipart("enc.txt", &log)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "decryption_failed");
        assert_eq!(mock.calls.load(Ordering::SeqCst), attempt);
    }
}

#[tokio::test]
async fn unreachable_dji_api_is_a_gateway_error() {
    // Port 9 (discard) on localhost: connection refused.
    let app = app_with(Some(GOOD_KEY), "http://127.0.0.1:9/keychains").await;
    let log = SynthFlight::demo().to_encrypted_bytes(&SynthKeys::new(1));
    let (status, body) = send_json(&app, multipart("enc.txt", &log)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"]["code"], "network_error");
    // The underlying cause is reported, the key is not.
    let message = body["error"]["message"].as_str().unwrap().to_lowercase();
    assert!(message.contains("connect"), "{message}");
    assert!(!body.to_string().contains(GOOD_KEY));
}

#[tokio::test]
async fn config_reports_key_presence_but_never_the_key() {
    let app = app_with(Some(GOOD_KEY), "http://127.0.0.1:9/unused").await;
    let (status, body) = send_json(&app, get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["apiKeyConfigured"], true);
    assert_eq!(body["supportedLogVersions"]["max"], 14);
    assert!(!body.to_string().contains(GOOD_KEY));

    let (_, _, headers) = send(&app, get("/api/config")).await;
    assert_eq!(headers["cache-control"], "no-cache");

    let (status, body) = send_json(&app, get("/api/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn reupload_reparses_flights_from_older_parser_versions() {
    let pool = db::connect_memory().await.unwrap();
    let app = app(AppState::new(
        pool.clone(),
        config(None, "http://127.0.0.1:9/unused"),
    ));
    let log = SynthFlight::demo().to_bytes();
    let (status, body) = send_json(&app, multipart("a.txt", &log)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["flight"]["id"].as_i64().unwrap();

    // Current version: a plain duplicate.
    let (_, body) = send_json(&app, multipart("a.txt", &log)).await;
    assert_eq!(body["reparsed"], false);

    // Simulate a flight stored by an older release without camera data.
    sqlx::query("UPDATE flights SET parse_version = 1 WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE samples SET gimbal_pitch_deg = 0, is_photo = 0")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM events")
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = send_json(&app, multipart("a.txt", &log)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"], false);
    assert_eq!(body["reparsed"], true);
    assert_eq!(body["flight"]["id"].as_i64().unwrap(), id, "id is kept");

    let (_, tel) = send_json(&app, get(&format!("/api/flights/{id}/telemetry"))).await;
    assert_eq!(tel["gimbalPitchDeg"][300], -90.0);
    assert_eq!(tel["isPhoto"][250], true);
    assert_eq!(tel["t"].as_array().unwrap().len(), 600);
    let (_, detail) = send_json(&app, get(&format!("/api/flights/{id}"))).await;
    let events = detail["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["message"] == "Photo taken"));
    let (_, list) = send_json(&app, get("/api/flights")).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}
