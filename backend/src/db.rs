use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;

use crate::error::{AppError, AppResult};
use crate::flight::{Event, EventLevel, FlightMeta, ParsedFlight, Sample};
use crate::keychain::Keychains;

pub async fn connect(data_dir: &Path) -> Result<SqlitePool, sqlx::Error> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("flights.db");
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// In-memory database for tests.
pub async fn connect_memory() -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    // A single connection: each in-memory connection is a separate database.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// A row of the flight list / the header of the flight view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightSummary {
    pub id: i64,
    pub file_name: String,
    pub file_size: i64,
    pub uploaded_at: String,
    #[serde(flatten)]
    pub meta: FlightMeta,
}

/// Id and `parse_version` of the flight imported from a file with this hash.
pub async fn find_by_hash(pool: &SqlitePool, sha256: &str) -> AppResult<Option<(i64, i64)>> {
    let row = sqlx::query("SELECT id, parse_version FROM flights WHERE file_sha256 = ?")
        .bind(sha256)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| (r.get::<i64, _>(0), r.get::<i64, _>(1))))
}

pub async fn insert_flight(
    pool: &SqlitePool,
    file_name: &str,
    sha256: &str,
    file_size: usize,
    parse_version: i64,
    flight: &ParsedFlight,
) -> AppResult<i64> {
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query(
        "INSERT INTO flights (file_name, file_sha256, file_size, uploaded_at, log_version, encrypted,
            aircraft_name, aircraft_sn, product_type, app_platform, app_version, start_time,
            duration_s, distance_m, max_height_m, max_h_speed_ms, max_v_speed_ms,
            home_lat, home_lon, takeoff_lat, takeoff_lon, landing_lat, landing_lon,
            location, sample_count, parse_version)
         VALUES (?, ?, ?, ?, 0, 0, '', '', '', '', '', NULL, 0, 0, 0, 0, 0,
            NULL, NULL, NULL, NULL, NULL, NULL, '', 0, ?)
         RETURNING id",
    )
    .bind(file_name)
    .bind(sha256)
    .bind(file_size as i64)
    .bind(Utc::now().to_rfc3339())
    .bind(parse_version)
    .fetch_one(&mut *tx)
    .await?
    .get(0);
    write_flight_data(&mut tx, id, flight).await?;
    tx.commit().await?;
    Ok(id)
}

/// Replaces the parsed data of an existing flight (re-import after the
/// parser/mapping changed), keeping its id and upload metadata.
pub async fn replace_flight(
    pool: &SqlitePool,
    id: i64,
    parse_version: i64,
    flight: &ParsedFlight,
) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM samples WHERE flight_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM events WHERE flight_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE flights SET parse_version = ? WHERE id = ?")
        .bind(parse_version)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    write_flight_data(&mut tx, id, flight).await?;
    tx.commit().await?;
    Ok(())
}

/// Writes the summary columns, samples and events of flight `id`.
async fn write_flight_data(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
    flight: &ParsedFlight,
) -> AppResult<()> {
    let m = &flight.meta;
    sqlx::query(
        "UPDATE flights SET log_version = ?, encrypted = ?, aircraft_name = ?, aircraft_sn = ?,
            product_type = ?, app_platform = ?, app_version = ?, start_time = ?,
            duration_s = ?, distance_m = ?, max_height_m = ?, max_h_speed_ms = ?, max_v_speed_ms = ?,
            home_lat = ?, home_lon = ?, takeoff_lat = ?, takeoff_lon = ?, landing_lat = ?,
            landing_lon = ?, location = ?, sample_count = ?
         WHERE id = ?",
    )
    .bind(m.log_version as i64)
    .bind(m.encrypted)
    .bind(&m.aircraft_name)
    .bind(&m.aircraft_sn)
    .bind(&m.product_type)
    .bind(&m.app_platform)
    .bind(&m.app_version)
    .bind(m.start_time.map(|t| t.to_rfc3339()))
    .bind(m.duration_s)
    .bind(m.distance_m)
    .bind(m.max_height_m)
    .bind(m.max_h_speed_ms)
    .bind(m.max_v_speed_ms)
    .bind(m.home_lat)
    .bind(m.home_lon)
    .bind(m.takeoff_lat)
    .bind(m.takeoff_lon)
    .bind(m.landing_lat)
    .bind(m.landing_lon)
    .bind(&m.location)
    .bind(m.sample_count as i64)
    .bind(id)
    .execute(&mut **tx)
    .await?;

    for (idx, s) in flight.samples.iter().enumerate() {
        sqlx::query(
            "INSERT INTO samples (flight_id, idx, t, timestamp_ms, lat, lon, height_m, altitude_m,
                h_speed_ms, v_speed_ms, yaw_deg, pitch_deg, roll_deg, battery_pct, battery_v,
                gps_sats, rc_uplink_pct, rc_downlink_pct, flight_mode, is_flying,
                gimbal_pitch_deg, is_photo, is_recording)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(idx as i64)
        .bind(s.t)
        .bind(s.timestamp_ms)
        .bind(s.lat)
        .bind(s.lon)
        .bind(s.height_m)
        .bind(s.altitude_m)
        .bind(s.h_speed_ms)
        .bind(s.v_speed_ms)
        .bind(s.yaw_deg)
        .bind(s.pitch_deg)
        .bind(s.roll_deg)
        .bind(s.battery_pct)
        .bind(s.battery_v)
        .bind(s.gps_sats)
        .bind(s.rc_uplink_pct)
        .bind(s.rc_downlink_pct)
        .bind(&s.flight_mode)
        .bind(s.is_flying)
        .bind(s.gimbal_pitch_deg)
        .bind(s.is_photo)
        .bind(s.is_recording)
        .execute(&mut **tx)
        .await?;
    }

    for (idx, e) in flight.events.iter().enumerate() {
        sqlx::query(
            "INSERT INTO events (flight_id, idx, t, level, message) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(idx as i64)
        .bind(e.t)
        .bind(e.level.as_str())
        .bind(&e.message)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

macro_rules! summary_columns {
    () => {
        "id, file_name, file_size, uploaded_at, log_version, encrypted,
    aircraft_name, aircraft_sn, product_type, app_platform, app_version, start_time,
    duration_s, distance_m, max_height_m, max_h_speed_ms, max_v_speed_ms,
    home_lat, home_lon, takeoff_lat, takeoff_lon, landing_lat, landing_lon,
    location, sample_count"
    };
}

fn summary_from_row(r: &sqlx::sqlite::SqliteRow) -> FlightSummary {
    let start_time: Option<String> = r.get("start_time");
    FlightSummary {
        id: r.get("id"),
        file_name: r.get("file_name"),
        file_size: r.get("file_size"),
        uploaded_at: r.get("uploaded_at"),
        meta: FlightMeta {
            log_version: r.get::<i64, _>("log_version") as u8,
            encrypted: r.get("encrypted"),
            aircraft_name: r.get("aircraft_name"),
            aircraft_sn: r.get("aircraft_sn"),
            product_type: r.get("product_type"),
            app_platform: r.get("app_platform"),
            app_version: r.get("app_version"),
            start_time: start_time
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
            duration_s: r.get("duration_s"),
            distance_m: r.get("distance_m"),
            max_height_m: r.get("max_height_m"),
            max_h_speed_ms: r.get("max_h_speed_ms"),
            max_v_speed_ms: r.get("max_v_speed_ms"),
            home_lat: r.get("home_lat"),
            home_lon: r.get("home_lon"),
            takeoff_lat: r.get("takeoff_lat"),
            takeoff_lon: r.get("takeoff_lon"),
            landing_lat: r.get("landing_lat"),
            landing_lon: r.get("landing_lon"),
            location: r.get("location"),
            sample_count: r.get::<i64, _>("sample_count") as usize,
        },
    }
}

pub async fn list_flights(pool: &SqlitePool) -> AppResult<Vec<FlightSummary>> {
    let rows = sqlx::query(concat!(
        "SELECT ",
        summary_columns!(),
        " FROM flights ORDER BY COALESCE(start_time, uploaded_at) DESC, id DESC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(summary_from_row).collect())
}

pub async fn get_flight(pool: &SqlitePool, id: i64) -> AppResult<FlightSummary> {
    let row = sqlx::query(concat!(
        "SELECT ",
        summary_columns!(),
        " FROM flights WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(summary_from_row(&row))
}

pub async fn delete_flight(pool: &SqlitePool, id: i64) -> AppResult<()> {
    let res = sqlx::query("DELETE FROM flights WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn get_samples(pool: &SqlitePool, id: i64) -> AppResult<Vec<Sample>> {
    let rows = sqlx::query(
        "SELECT t, timestamp_ms, lat, lon, height_m, altitude_m, h_speed_ms, v_speed_ms,
            yaw_deg, pitch_deg, roll_deg, battery_pct, battery_v, gps_sats,
            rc_uplink_pct, rc_downlink_pct, flight_mode, is_flying,
            gimbal_pitch_deg, is_photo, is_recording
         FROM samples WHERE flight_id = ? ORDER BY idx",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Sample {
            t: r.get("t"),
            timestamp_ms: r.get("timestamp_ms"),
            lat: r.get("lat"),
            lon: r.get("lon"),
            height_m: r.get("height_m"),
            altitude_m: r.get("altitude_m"),
            h_speed_ms: r.get("h_speed_ms"),
            v_speed_ms: r.get("v_speed_ms"),
            yaw_deg: r.get("yaw_deg"),
            pitch_deg: r.get("pitch_deg"),
            roll_deg: r.get("roll_deg"),
            battery_pct: r.get("battery_pct"),
            battery_v: r.get("battery_v"),
            gps_sats: r.get("gps_sats"),
            rc_uplink_pct: r.get("rc_uplink_pct"),
            rc_downlink_pct: r.get("rc_downlink_pct"),
            flight_mode: r.get("flight_mode"),
            is_flying: r.get("is_flying"),
            gimbal_pitch_deg: r.get("gimbal_pitch_deg"),
            is_photo: r.get("is_photo"),
            is_recording: r.get("is_recording"),
        })
        .collect())
}

pub async fn get_events(pool: &SqlitePool, id: i64) -> AppResult<Vec<Event>> {
    let rows = sqlx::query("SELECT t, level, message FROM events WHERE flight_id = ? ORDER BY idx")
        .bind(id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| Event {
            t: r.get("t"),
            level: EventLevel::parse(r.get::<&str, _>("level")),
            message: r.get("message"),
        })
        .collect())
}

pub async fn cached_keychains(
    pool: &SqlitePool,
    request_sha256: &str,
) -> AppResult<Option<Keychains>> {
    let row = sqlx::query("SELECT keychains_json FROM keychain_cache WHERE request_sha256 = ?")
        .bind(request_sha256)
        .fetch_optional(pool)
        .await?;
    match row {
        // A corrupt cache entry is ignored and re-fetched.
        Some(r) => Ok(serde_json::from_str(r.get::<&str, _>(0)).ok()),
        None => Ok(None),
    }
}

pub async fn store_keychains(
    pool: &SqlitePool,
    request_sha256: &str,
    keychains: &Keychains,
) -> AppResult<()> {
    let json = serde_json::to_string(keychains).map_err(|e| AppError::Internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO keychain_cache (request_sha256, keychains_json, created_at) VALUES (?, ?, ?)
         ON CONFLICT (request_sha256) DO UPDATE SET keychains_json = excluded.keychains_json,
            created_at = excluded.created_at",
    )
    .bind(request_sha256)
    .bind(json)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_keychains(pool: &SqlitePool, request_sha256: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM keychain_cache WHERE request_sha256 = ?")
        .bind(request_sha256)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cached obstacle GeoJSON for a flight and source, with its fetch time.
pub async fn cached_obstacles(
    pool: &SqlitePool,
    flight_id: i64,
    source: &str,
) -> AppResult<Option<(String, String)>> {
    let row = sqlx::query(
        "SELECT geojson, fetched_at FROM flight_obstacles WHERE flight_id = ? AND source = ?",
    )
    .bind(flight_id)
    .bind(source)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

pub async fn store_obstacles(
    pool: &SqlitePool,
    flight_id: i64,
    source: &str,
    geojson: &str,
) -> AppResult<String> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO flight_obstacles (flight_id, source, geojson, fetched_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (flight_id, source) DO UPDATE SET geojson = excluded.geojson,
            fetched_at = excluded.fetched_at",
    )
    .bind(flight_id)
    .bind(source)
    .bind(geojson)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(now)
}
