//! Obstacle layers against mock Overpass and GUGiK WCS servers.

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

use dji_log_viewer::config::Config;
use dji_log_viewer::obstacles::puwg92;
use dji_log_viewer::obstacles::raster::{Grid, encode_geotiff};
use dji_log_viewer::synthetic::SynthFlight;
use dji_log_viewer::{AppState, app, db};

#[derive(Clone, Default)]
struct Calls {
    overpass: Arc<AtomicUsize>,
    wcs: Arc<AtomicUsize>,
}

/// GUGiK-like WCS: 1 m grids for the requested box; ground at 100 m, and in
/// the surface model a 15 m object 20..30 m east of the take-off point.
async fn mock_services() -> (String, Calls) {
    let calls = Calls::default();
    let (te, tn) = puwg92::forward(21.0122, 52.2297);

    async fn overpass(State(c): State<Calls>, body: String) -> Json<Value> {
        c.overpass.fetch_add(1, Ordering::SeqCst);
        assert!(body.contains("natural"), "{body}");
        Json(json!({ "elements": [
            { "type": "node", "id": 1, "lat": 52.2298, "lon": 21.0125, "tags": { "natural": "tree", "height": "12 m", "diameter_crown": "6" } },
            { "type": "node", "id": 2, "lat": 52.2296, "lon": 21.0130, "tags": { "natural": "tree" } }
        ]}))
    }

    let wcs = move |surface: bool| {
        move |State(c): State<Calls>, Query(q): Query<HashMap<String, f64>>| async move {
            c.wcs.fetch_add(1, Ordering::SeqCst);
            let (e0, n0, e1, n1) = (q["e0"], q["n0"], q["e1"], q["n1"]);
            let (w, h) = ((e1 - e0) as usize, (n1 - n0) as usize);
            let mut data = vec![100.0f32; w * h];
            if surface {
                for r in 0..h {
                    for col in 0..w {
                        let (e, n) = (e0 + col as f64 + 0.5, n1 - r as f64 - 0.5);
                        if (te + 20.0..te + 30.0).contains(&e) && (tn - 5.0..tn + 5.0).contains(&n)
                        {
                            data[r * w + col] = 115.0;
                        }
                    }
                }
            }
            if surface {
                // Like GUGiK's NMPT: an Arc/Info ASCII grid, here northing-first
                // (columns run north, the first row is the easternmost).
                let mut text = format!(
                    "ncols {h}\nnrows {w}\nxllcorner {n0}\nyllcorner {e0}\ncellsize 1\nNODATA_value -9999\n"
                );
                for col in (0..w).rev() {
                    let row: Vec<String> = (0..h)
                        .rev()
                        .map(|r| data[r * w + col].to_string())
                        .collect();
                    text.push_str(&row.join(" "));
                    text.push('\n');
                }
                // Wrapped like GUGiK's WCS 2.0 multipart/related answer.
                let body = format!(
                    "--wcs\r\nContent-Type: text/xml\r\nContent-ID: wcs\r\n\r\n<gml:RectifiedGridCoverage/>\r\n\
                     --wcs\r\nContent-Type: image/x-aaigrid\r\nContent-Description: coverage data\r\n\
                     Content-Transfer-Encoding: binary\r\nContent-ID: coverage/out.asc\r\n\r\n{text}\r\n--wcs--\r\n"
                );
                return (
                    [("content-type", "multipart/related; boundary=wcs")],
                    body.into_bytes(),
                );
            }
            let g = Grid {
                width: w,
                height: h,
                left: e0,
                top: n1,
                px: 1.0,
                py: 1.0,
                data,
                nodata: None,
            };
            ([("content-type", "image/tiff")], encode_geotiff(&g))
        }
    };

    let router = Router::new()
        .route("/overpass", post(overpass))
        .route("/nmt", get(wcs(false)))
        .route("/nmpt", get(wcs(true)))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), calls)
}

fn config(base: Option<&str>) -> Config {
    let tpl = |path: &str| {
        base.map(|b| format!("{b}/{path}?e0={{minE}}&n0={{minN}}&e1={{maxE}}&n1={{maxN}}"))
    };
    Config {
        api_key: None,
        data_dir: "/nonexistent".into(),
        static_dir: "/nonexistent".into(),
        port: 0,
        max_upload_bytes: 5 * 1024 * 1024,
        keychain_endpoint: "http://127.0.0.1:9/unused".into(),
        map_tile_url: "t".into(),
        map_attribution: "t".into(),
        terrain_url: None,
        terrain_encoding: "terrarium".into(),
        terrain_attribution: "t".into(),
        vector_tiles_url: None,
        overpass_url: base.map(|b| format!("{b}/overpass")),
        gugik_nmt_url: tpl("nmt"),
        gugik_nmpt_url: tpl("nmpt"),
    }
}

async fn setup(base: Option<&str>) -> (axum::Router, i64) {
    let pool = db::connect_memory().await.unwrap();
    let app = app(AppState::new(pool, config(base)));
    let log = SynthFlight::demo().to_bytes();
    let boundary = "b";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f.txt\"\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&log);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::post("/api/flights")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let (_, v) = send(&app, req).await;
    let id = v["flight"]["id"].as_i64().unwrap();
    (app, id)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn osm_trees_are_fetched_once_and_cached() {
    let (base, calls) = mock_services().await;
    let (app, id) = setup(Some(&base)).await;

    let (status, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/trees")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["cached"], false);
    let f = v["data"]["features"].as_array().unwrap();
    assert_eq!(f.len(), 2);
    assert_eq!(f[0]["properties"]["height"], 12.0);
    assert_eq!(f[0]["properties"]["crown"], 6.0);
    assert!(f[1]["properties"]["height"].is_null());

    let (_, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/trees")).await;
    assert_eq!(v["cached"], true);
    assert_eq!(calls.overpass.load(Ordering::SeqCst), 1);

    let (_, v) = get_json(
        &app,
        &format!("/api/flights/{id}/obstacles/trees?refresh=true"),
    )
    .await;
    assert_eq!(v["cached"], false);
    assert_eq!(calls.overpass.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lidar_heights_become_blocks_relative_to_takeoff_ground() {
    let (base, calls) = mock_services().await;
    let (app, id) = setup(Some(&base)).await;

    let (status, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/lidar")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(calls.wcs.load(Ordering::SeqCst), 2, "NMT + NMPT");
    let f = v["data"]["features"].as_array().unwrap();
    assert!(!f.is_empty());
    // Only the 15 m object, on ground level with the take-off point.
    assert!(f.iter().all(|x| x["properties"]["height"] == 15.0), "{f:?}");
    assert!(f.iter().all(|x| x["properties"]["groundRel"] == 0.0));
    // Blocks lie 20..30 m east of take-off.
    let lon0 = 21.0122;
    let m_per_deg = 111_320.0 * 52.2297f64.to_radians().cos();
    for x in f {
        for p in x["geometry"]["coordinates"][0].as_array().unwrap() {
            let east = (p[0].as_f64().unwrap() - lon0) * m_per_deg;
            assert!((19.0..31.0).contains(&east), "{east}");
        }
    }

    let (_, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/lidar")).await;
    assert_eq!(v["cached"], true);
    assert_eq!(calls.wcs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn disabled_and_unknown_sources() {
    let (app, id) = setup(None).await;
    let (status, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/lidar")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v["error"]["code"], "not_available");
    let (status, _) = get_json(&app, &format!("/api/flights/{id}/obstacles/pylons")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = get_json(&app, "/api/flights/999/obstacles/trees").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, cfg) = get_json(&app, "/api/config").await;
    assert_eq!(
        cfg["obstacleSources"],
        json!({ "trees": false, "lidar": false })
    );
}

#[tokio::test]
async fn upstream_failures_are_reported_and_not_cached() {
    // Nothing listens on port 9.
    let (app, id) = setup(Some("http://127.0.0.1:9")).await;
    let (status, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/trees")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{v}");
    assert_eq!(v["error"]["code"], "network_error");
    assert!(v["error"]["message"].as_str().unwrap().contains("Overpass"));

    // GUGiK: both services named, with the cause, and nothing about DJI.
    let (status, v) = get_json(&app, &format!("/api/flights/{id}/obstacles/lidar")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("GUGiK NMT") && msg.contains("GUGiK NMPT"),
        "{msg}"
    );
    assert!(msg.to_lowercase().contains("connect"), "{msg}");
    assert!(!msg.contains("DJI"), "{msg}");
}
