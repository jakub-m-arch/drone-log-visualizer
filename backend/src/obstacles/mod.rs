//! 3D obstacle data around a flight: OSM trees (Overpass API) and, in Poland,
//! object heights from GUGiK airborne laser scanning (NMPT − NMT).

pub mod puwg92;
pub mod raster;
pub mod sources;

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::AppState;
use crate::db;
use crate::error::{AppError, AppResult};
use sources::{BBox, LidarRequest};

/// Margin around the flight track for the OSM trees query.
const MARGIN_M: f64 = 100.0;
/// Largest area side requested from Overpass.
const MAX_TREES_SIDE_M: f64 = 1_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// A LiDAR fetch running in the background (GUGiK can take minutes).
pub struct Job {
    pub progress: Arc<sources::Progress>,
    /// Set when the job failed; taken (and the job removed) when reported.
    pub error: Option<AppError>,
}

pub type Jobs = Arc<Mutex<HashMap<i64, Job>>>;

/// Result of an obstacle request: data, or a LiDAR fetch still in progress.
pub enum Outcome {
    Ready(Value),
    Loading { done: usize, total: usize },
}

fn ready(source: Source, fetched_at: String, cached: bool, data: Value) -> Outcome {
    Outcome::Ready(
        json!({ "source": source.as_str(), "fetchedAt": fetched_at, "cached": cached, "data": data }),
    )
}

/// Obstacles for a flight, from the cache or fetched (and cached) on demand.
/// Trees are fetched within the request; LiDAR runs as a background job and
/// the caller polls until it is ready.
pub async fn for_flight(
    state: &AppState,
    flight_id: i64,
    source: Source,
    refresh: bool,
) -> AppResult<Outcome> {
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

    if source == Source::Lidar {
        let mut jobs = state.obstacle_jobs.lock().expect("jobs lock");
        if let Some(job) = jobs.get_mut(&flight_id) {
            if let Some(err) = job.error.take() {
                jobs.remove(&flight_id);
                return Err(err);
            }
            return Ok(Outcome::Loading {
                done: job.progress.done.load(Ordering::SeqCst),
                total: job.progress.total.load(Ordering::SeqCst),
            });
        }
    }

    if !refresh
        && let Some((geojson, fetched_at)) =
            db::cached_obstacles(&state.pool, flight_id, source.as_str()).await?
    {
        let data: Value =
            serde_json::from_str(&geojson).map_err(|e| AppError::Internal(e.to_string()))?;
        return Ok(ready(source, fetched_at, true, data));
    }

    let samples = db::get_samples(&state.pool, flight_id).await?;
    let points: Vec<(f64, f64)> = samples
        .iter()
        .filter_map(|s| Some((s.lon?, s.lat?)))
        .collect();
    if points.is_empty() {
        return Err(AppError::NotAvailable(
            "This flight has no GPS track.".into(),
        ));
    }

    match source {
        Source::Trees => {
            let centre = match (flight.meta.takeoff_lon, flight.meta.takeoff_lat) {
                (Some(lon), Some(lat)) => (lon, lat),
                _ => points[0],
            };
            let (near, _) = sources::within_radius(&points, centre, sources::RADIUS_M);
            let near = if near.is_empty() { vec![centre] } else { near };
            let bbox = BBox::around(&near, MARGIN_M).expect("points not empty");
            let data = sources::osm_trees(
                cfg.overpass_url.as_deref().unwrap_or_default(),
                bbox.clamp_size(MAX_TREES_SIDE_M),
            )
            .await?;
            let count = data["features"].as_array().map_or(0, Vec::len);
            tracing::info!(flight_id, source = "trees", count, "fetched obstacles");
            let fetched_at =
                db::store_obstacles(&state.pool, flight_id, "trees", &data.to_string()).await?;
            Ok(ready(source, fetched_at, false, data))
        }
        Source::Lidar => {
            if !points.iter().all(|&(lon, lat)| puwg92::in_poland(lon, lat)) {
                return Err(AppError::NotAvailable(
                    "LiDAR heights from GUGiK are only available for flights in Poland.".into(),
                ));
            }
            let reference = match (flight.meta.takeoff_lon, flight.meta.takeoff_lat) {
                (Some(lon), Some(lat)) => (lon, lat),
                _ => points[0],
            };
            let progress = Arc::new(sources::Progress::default());
            state.obstacle_jobs.lock().expect("jobs lock").insert(
                flight_id,
                Job {
                    progress: progress.clone(),
                    error: None,
                },
            );
            let state = state.clone();
            tokio::spawn(async move {
                let cfg = &state.config;
                let result = sources::gugik_lidar(LidarRequest {
                    dtm_template: cfg.gugik_nmt_url.as_deref().unwrap_or_default(),
                    dsm_template: cfg.gugik_nmpt_url.as_deref().unwrap_or_default(),
                    track: &points,
                    reference,
                    progress,
                })
                .await;
                let stored = match result {
                    Ok(data) => {
                        let count = data["features"].as_array().map_or(0, Vec::len);
                        tracing::info!(flight_id, source = "lidar", count, "fetched obstacles");
                        db::store_obstacles(&state.pool, flight_id, "lidar", &data.to_string())
                            .await
                            .map(|_| ())
                    }
                    Err(e) => Err(e),
                };
                let mut jobs = state.obstacle_jobs.lock().expect("jobs lock");
                match stored {
                    Ok(()) => {
                        jobs.remove(&flight_id);
                    }
                    Err(e) => {
                        tracing::warn!(flight_id, code = e.code(), "LiDAR fetch failed: {e}");
                        if let Some(job) = jobs.get_mut(&flight_id) {
                            job.error = Some(e);
                        }
                    }
                }
            });
            Ok(Outcome::Loading { done: 0, total: 0 })
        }
    }
}
