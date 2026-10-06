use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::json;

use crate::AppState;
use crate::db::{self, FlightSummary};
use crate::error::{AppError, AppResult};
use crate::export;
use crate::flight::Event;
use crate::ingest::{self, FIRST_ENCRYPTED_VERSION, MAX_SUPPORTED_VERSION};

pub fn router(state: AppState) -> Router {
    // Leave headroom for multipart framing on top of the file itself.
    let body_limit = state.config.max_upload_bytes + 1024 * 1024;
    Router::new()
        .route("/api/config", get(config))
        .route(
            "/api/flights",
            get(list_flights)
                .post(upload)
                .layer(DefaultBodyLimit::max(body_limit)),
        )
        .route("/api/flights/{id}", get(get_flight).delete(delete_flight))
        .route("/api/flights/{id}/telemetry", get(telemetry))
        .route("/api/flights/{id}/export/{format}", get(export_flight))
        .route(
            "/api/flights/{id}/obstacles/{source}",
            get(flight_obstacles),
        )
        .route("/api/{*rest}", get(api_not_found).post(api_not_found))
        .with_state(state)
}

async fn api_not_found() -> AppError {
    AppError::NotFound
}

async fn config(State(state): State<AppState>) -> Json<serde_json::Value> {
    let c = &state.config;
    Json(json!({
        "appVersion": env!("CARGO_PKG_VERSION"),
        "parserVersion": crate::PARSER_VERSION,
        "apiKeyConfigured": c.api_key.is_some(),
        "supportedLogVersions": { "min": 1, "max": MAX_SUPPORTED_VERSION },
        "encryptedFromVersion": FIRST_ENCRYPTED_VERSION,
        "maxUploadMb": c.max_upload_bytes / 1024 / 1024,
        "mapTileUrl": c.map_tile_url,
        "mapAttribution": c.map_attribution,
        "terrainUrl": c.terrain_url,
        "terrainEncoding": c.terrain_encoding,
        "terrainAttribution": c.terrain_attribution,
        "vectorTilesUrl": c.vector_tiles_url,
        "obstacleSources": {
            "trees": c.overpass_url.is_some(),
            "lidar": c.gugik_nmt_url.is_some() && c.gugik_nmpt_url.is_some(),
        },
    }))
}

async fn list_flights(State(state): State<AppState>) -> AppResult<Json<Vec<FlightSummary>>> {
    Ok(Json(db::list_flights(&state.pool).await?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    created: bool,
    reparsed: bool,
    flight: FlightSummary,
}

async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> AppResult<Response> {
    let limit_mb = state.config.max_upload_bytes / 1024 / 1024;
    let map_err = |e: axum::extract::multipart::MultipartError| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::TooLarge(limit_mb)
        } else {
            AppError::BadRequest(format!("Invalid upload: {}", e.body_text()))
        }
    };

    while let Some(field) = multipart.next_field().await.map_err(map_err)? {
        if field.name() != Some("file") {
            continue;
        }
        let file_name = sanitize_file_name(field.file_name().unwrap_or("flight-log.txt"));
        let bytes = field.bytes().await.map_err(map_err)?;
        if bytes.len() > state.config.max_upload_bytes {
            return Err(AppError::TooLarge(limit_mb));
        }
        let outcome = ingest::ingest(&state, &file_name, bytes.to_vec()).await?;
        let flight = db::get_flight(&state.pool, outcome.id).await?;
        let status = if outcome.created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };
        return Ok((
            status,
            Json(UploadResponse {
                created: outcome.created,
                reparsed: outcome.reparsed,
                flight,
            }),
        )
            .into_response());
    }
    Err(AppError::BadRequest(
        "Missing multipart field \"file\".".into(),
    ))
}

fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if cleaned.trim().is_empty() {
        "flight-log.txt".into()
    } else {
        cleaned
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FlightDetail {
    flight: FlightSummary,
    events: Vec<Event>,
}

async fn get_flight(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<FlightDetail>> {
    let flight = db::get_flight(&state.pool, id).await?;
    let events = db::get_events(&state.pool, id).await?;
    Ok(Json(FlightDetail { flight, events }))
}

async fn delete_flight(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    db::delete_flight(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Column-oriented telemetry (one array per series), which is what uPlot
/// consumes and is ~3x smaller than an array of objects.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Telemetry {
    t: Vec<f64>,
    timestamp_ms: Vec<Option<i64>>,
    lat: Vec<Option<f64>>,
    lon: Vec<Option<f64>>,
    height_m: Vec<f64>,
    altitude_m: Vec<f64>,
    h_speed_ms: Vec<f64>,
    v_speed_ms: Vec<f64>,
    yaw_deg: Vec<f64>,
    battery_pct: Vec<Option<f64>>,
    battery_v: Vec<Option<f64>>,
    gps_sats: Vec<i64>,
    rc_uplink_pct: Vec<Option<f64>>,
    rc_downlink_pct: Vec<Option<f64>>,
    flight_mode: Vec<String>,
    is_flying: Vec<bool>,
    gimbal_pitch_deg: Vec<f64>,
    is_photo: Vec<bool>,
    is_recording: Vec<bool>,
}

async fn telemetry(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Telemetry>> {
    db::get_flight(&state.pool, id).await?;
    let samples = db::get_samples(&state.pool, id).await?;
    let mut out = Telemetry::default();
    for s in samples {
        out.t.push(s.t);
        out.timestamp_ms.push(s.timestamp_ms);
        out.lat.push(s.lat);
        out.lon.push(s.lon);
        out.height_m.push(s.height_m);
        out.altitude_m.push(s.altitude_m);
        out.h_speed_ms.push(s.h_speed_ms);
        out.v_speed_ms.push(s.v_speed_ms);
        out.yaw_deg.push(s.yaw_deg);
        out.battery_pct.push(s.battery_pct);
        out.battery_v.push(s.battery_v);
        out.gps_sats.push(s.gps_sats);
        out.rc_uplink_pct.push(s.rc_uplink_pct);
        out.rc_downlink_pct.push(s.rc_downlink_pct);
        out.flight_mode.push(s.flight_mode);
        out.is_flying.push(s.is_flying);
        out.gimbal_pitch_deg.push(s.gimbal_pitch_deg);
        out.is_photo.push(s.is_photo);
        out.is_recording.push(s.is_recording);
    }
    Ok(Json(out))
}

#[derive(serde::Deserialize)]
struct ObstacleQuery {
    #[serde(default)]
    refresh: bool,
}

async fn flight_obstacles(
    State(state): State<AppState>,
    Path((id, source)): Path<(i64, String)>,
    axum::extract::Query(q): axum::extract::Query<ObstacleQuery>,
) -> AppResult<Response> {
    use crate::obstacles::{Outcome, Source, for_flight};
    let source = Source::parse(&source)?;
    Ok(match for_flight(&state, id, source, q.refresh).await? {
        Outcome::Ready(v) => Json(v).into_response(),
        // LiDAR still loading in the background: the client polls.
        Outcome::Loading { done, total } => (
            StatusCode::ACCEPTED,
            Json(json!({
                "source": source.as_str(),
                "status": "loading",
                "done": done,
                "total": total,
            })),
        )
            .into_response(),
    })
}

async fn export_flight(
    State(state): State<AppState>,
    Path((id, format)): Path<(i64, String)>,
) -> AppResult<Response> {
    let flight = db::get_flight(&state.pool, id).await?;
    let samples = db::get_samples(&state.pool, id).await?;
    let (body, mime, ext) = match format.as_str() {
        "csv" => (export::csv(&samples), "text/csv; charset=utf-8", "csv"),
        "gpx" => (export::gpx(&flight, &samples), "application/gpx+xml", "gpx"),
        "kml" => (
            export::kml(&flight, &samples),
            "application/vnd.google-earth.kml+xml",
            "kml",
        ),
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown export format {other:?} (use csv, gpx or kml)."
            )));
        }
    };
    let disposition = format!(
        "attachment; filename=\"{}.{ext}\"",
        export::file_stem(&flight)
    );
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::sanitize_file_name;

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_file_name("C:\\logs\\DJIFlightRecord_1.txt"),
            "DJIFlightRecord_1.txt"
        );
        assert_eq!(sanitize_file_name(""), "flight-log.txt");
    }
}
