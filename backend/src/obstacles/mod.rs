//! 3D obstacle data around a flight: OSM trees (Overpass API) and, in Poland,
//! object heights from GUGiK airborne laser scanning (NMPT − NMT).

pub mod puwg92;
pub mod raster;
pub mod sources;

use serde_json::{Value, json};

use crate::AppState;
use crate::db;
use crate::error::{AppError, AppResult};
use sources::{BBox, LidarRequest};

/// Margin around the flight track for obstacle queries.
const MARGIN_M: f64 = 100.0;
/// Largest area side requested from Overpass.
const MAX_TREES_SIDE_M: f64 = 1_000.0;
/// Largest area side requested from GUGiK (NMPT is 0.5 m: 4 M cells per km²).
const MAX_LIDAR_SIDE_M: f64 = 1_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Trees,
    Lidar,
}

impl Source {
    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "trees" => Ok(Source::Trees),
            "lidar" => Ok(Source::Lidar),
            other => Err(AppError::BadRequest(format!(
                "Unknown obstacle source {other:?} (use trees or lidar)."
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Trees => "trees",
            Source::Lidar => "lidar",
        }
    }
}

/// Obstacles for a flight, from the cache or fetched (and cached) on demand.
pub async fn for_flight(
    state: &AppState,
    flight_id: i64,
    source: Source,
    refresh: bool,
) -> AppResult<Value> {
    let flight = db::get_flight(&state.pool, flight_id).await?;
    let cfg = &state.config;
    let configured = match source {
        Source::Trees => cfg.overpass_url.is_some(),
        Source::Lidar => cfg.gugik_nmt_url.is_some() && cfg.gugik_nmpt_url.is_some(),
    };
    if !configured {
        return Err(AppError::NotAvailable(format!(
            "The {} source is disabled in the server configuration.",
            source.as_str()
        )));
    }

    if !refresh
        && let Some((geojson, fetched_at)) =
            db::cached_obstacles(&state.pool, flight_id, source.as_str()).await?
    {
        let data: Value =
            serde_json::from_str(&geojson).map_err(|e| AppError::Internal(e.to_string()))?;
        return Ok(
            json!({ "source": source.as_str(), "fetchedAt": fetched_at, "cached": true, "data": data }),
        );
    }

    let samples = db::get_samples(&state.pool, flight_id).await?;
    let points: Vec<(f64, f64)> = samples
        .iter()
        .filter_map(|s| Some((s.lon?, s.lat?)))
        .collect();
    let bbox = BBox::around(&points, MARGIN_M)
        .ok_or_else(|| AppError::NotAvailable("This flight has no GPS track.".into()))?;

    let data = match source {
        Source::Trees => {
            sources::osm_trees(
                cfg.overpass_url.as_deref().unwrap_or_default(),
                bbox.clamp_size(MAX_TREES_SIDE_M),
            )
            .await?
        }
        Source::Lidar => {
            let reference = match (flight.meta.takeoff_lon, flight.meta.takeoff_lat) {
                (Some(lon), Some(lat)) => (lon, lat),
                _ => points[0],
            };
            sources::gugik_lidar(LidarRequest {
                dtm_template: cfg.gugik_nmt_url.as_deref().unwrap_or_default(),
                dsm_template: cfg.gugik_nmpt_url.as_deref().unwrap_or_default(),
                bbox: bbox.clamp_size(MAX_LIDAR_SIDE_M),
                reference,
            })
            .await?
        }
    };
    let count = data["features"].as_array().map_or(0, Vec::len);
    tracing::info!(
        flight_id,
        source = source.as_str(),
        count,
        "fetched obstacles"
    );
    let fetched_at =
        db::store_obstacles(&state.pool, flight_id, source.as_str(), &data.to_string()).await?;
    Ok(json!({ "source": source.as_str(), "fetchedAt": fetched_at, "cached": false, "data": data }))
}
