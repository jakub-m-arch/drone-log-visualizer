//! Fetching obstacle data from external services.

use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::puwg92;
use super::raster::{self, BlockParams, Raster};
use crate::error::{AppError, AppResult};

/// Geographic bounding box in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl BBox {
    /// Bounding box of `points` (lon, lat), grown by `margin_m` on each side.
    pub fn around(points: &[(f64, f64)], margin_m: f64) -> Option<BBox> {
        let first = points.first()?;
        let mut b = BBox {
            west: first.0,
            east: first.0,
            south: first.1,
            north: first.1,
        };
        for &(lon, lat) in points {
            b.west = b.west.min(lon);
            b.east = b.east.max(lon);
            b.south = b.south.min(lat);
            b.north = b.north.max(lat);
        }
        let dlat = margin_m / 111_320.0;
        let dlon = margin_m / (111_320.0 * ((b.south + b.north) / 2.0).to_radians().cos());
        Some(BBox {
            west: b.west - dlon,
            east: b.east + dlon,
            south: b.south - dlat,
            north: b.north + dlat,
        })
    }

    /// Clamps the box to at most `max_m` per side around its centre.
    pub fn clamp_size(self, max_m: f64) -> BBox {
        let (clon, clat) = (
            (self.west + self.east) / 2.0,
            (self.south + self.north) / 2.0,
        );
        let half_lat = (max_m / 2.0) / 111_320.0;
        let half_lon = (max_m / 2.0) / (111_320.0 * clat.to_radians().cos());
        BBox {
            west: self.west.max(clon - half_lon),
            east: self.east.min(clon + half_lon),
            south: self.south.max(clat - half_lat),
            north: self.north.min(clat + half_lat),
        }
    }
}

fn client() -> reqwest::Client {
    client_with_timeout(Duration::from_secs(90))
}

fn client_with_timeout(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!(
            "dji-log-viewer/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/jakub-m-arch/drone-log-visualizer)"
        ))
        .build()
        .expect("HTTP client")
}

fn upstream(service: &str, e: reqwest::Error) -> AppError {
    AppError::Network(format!(
        "{service}: {}",
        crate::error::describe_request_error(e)
    ))
}

/// GET with up to 4 attempts when the request could not be sent (refused or
/// dropped connections, TLS EOF). GUGiK drops a share of connections even
/// from curl, so another attempt usually succeeds. HTTP errors and timeouts
/// are not retried.
async fn get_with_retry(service: &str, url: &str) -> AppResult<reqwest::Response> {
    const ATTEMPTS: u32 = 4;
    let mut attempt = 1;
    loop {
        // GUGiK can take a while to render a grid.
        match client_with_timeout(Duration::from_secs(180))
            .get(url)
            .send()
            .await
        {
            Ok(res) => return Ok(res),
            Err(e)
                if attempt < ATTEMPTS && !e.is_timeout() && (e.is_connect() || e.is_request()) =>
            {
                tracing::warn!(
                    service,
                    attempt,
                    "request failed, retrying: {}",
                    crate::error::describe_request_error(e)
                );
                tokio::time::sleep(Duration::from_millis(1500 * u64::from(attempt))).await;
                attempt += 1;
            }
            Err(e) => return Err(upstream(service, e)),
        }
    }
}

// ---- OSM trees via Overpass -------------------------------------------------

#[derive(Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassNode>,
}

#[derive(Deserialize)]
struct OverpassNode {
    lat: Option<f64>,
    lon: Option<f64>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
}

/// Parses OSM length values such as "12", "12 m", "12.5m", "40 ft".
pub fn parse_length_m(v: &str) -> Option<f64> {
    let v = v.trim().to_lowercase().replace(',', ".");
    let (num, factor) = if let Some(n) = v.strip_suffix("ft") {
        (n, 0.3048)
    } else if let Some(n) = v.strip_suffix('m') {
        (n, 1.0)
    } else {
        (v.as_str(), 1.0)
    };
    let x: f64 = num.trim().parse().ok()?;
    (x.is_finite() && x > 0.0 && x < 150.0).then_some(x * factor)
}

/// Trees (`natural=tree` nodes) in the box, as GeoJSON points with
/// `height` / `crown` in metres when mapped.
/// Trees (`natural=tree` nodes) in the box, as GeoJSON points with
/// `height` / `crown` in metres when mapped.
///
/// `endpoints` is a comma-separated list of Overpass interpreters. Public
/// instances are often busy, so on 429/5xx or connection errors the next one
/// is tried, and the whole list once more after a pause.
pub async fn osm_trees(endpoints: &str, b: BBox) -> AppResult<Value> {
    let query = format!(
        "[out:json][timeout:25];node[\"natural\"=\"tree\"]({},{},{},{});out body 20000;",
        b.south, b.west, b.north, b.east
    );
    let endpoints: Vec<&str> = endpoints
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut errors: Vec<String> = Vec::new();
    let mut body: Option<OverpassResponse> = None;
    'rounds: for round in 0..2 {
        if round > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        for endpoint in &endpoints {
            let host = endpoint.split('/').nth(2).unwrap_or(endpoint);
            match client()
                .post(*endpoint)
                .form(&[("data", &query)])
                .send()
                .await
            {
                Ok(res) if res.status().is_success() => {
                    match res.json::<OverpassResponse>().await {
                        Ok(parsed) => {
                            body = Some(parsed);
                            break 'rounds;
                        }
                        Err(e) => errors.push(format!("{host}: unexpected response ({e})")),
                    }
                }
                Ok(res) => {
                    let status = res.status();
                    errors.push(format!("{host}: HTTP {status}"));
                    // Other client errors (e.g. a bad query) will not improve.
                    if status.is_client_error() && status.as_u16() != 429 {
                        break 'rounds;
                    }
                }
                Err(e) => errors.push(format!(
                    "{host}: {}",
                    crate::error::describe_request_error(e)
                )),
            }
            tracing::warn!(endpoint = host, "Overpass request failed, trying next");
        }
    }
    let Some(body) = body else {
        return Err(AppError::Upstream(format!(
            "Overpass API is busy or unreachable, try again later ({})",
            errors.join("; ")
        )));
    };
    let features: Vec<Value> = body
        .elements
        .into_iter()
        .filter_map(|n| {
            let (lat, lon) = (n.lat?, n.lon?);
            let height = n.tags.get("height").and_then(|v| parse_length_m(v));
            let crown = n.tags.get("diameter_crown").and_then(|v| parse_length_m(v));
            Some(json!({
                "type": "Feature",
                "properties": { "height": height, "crown": crown },
                "geometry": { "type": "Point", "coordinates": [lon, lat] },
            }))
        })
        .collect();
    Ok(json!({ "type": "FeatureCollection", "features": features }))
}

// ---- GUGiK NMT / NMPT (Poland) ----------------------------------------------

/// Fills a request template with the box in EPSG:2180 metres:
/// `{minE} {minN} {maxE} {maxN}` (easting / northing).
pub fn fill_template(template: &str, (min_e, min_n, max_e, max_n): (f64, f64, f64, f64)) -> String {
    template
        .replace("{minE}", &format!("{min_e:.1}"))
        .replace("{minN}", &format!("{min_n:.1}"))
        .replace("{maxE}", &format!("{max_e:.1}"))
        .replace("{maxN}", &format!("{max_n:.1}"))
}

/// Short plain-text summary of an error body: HTML tags and whitespace
/// collapsed (Apache error pages repeat the status), capped at 200 chars.
pub fn summarize_body(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(&bytes[..bytes.len().min(4000)]);
    let mut text = String::new();
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 200 {
        format!("{}…", text.chars().take(200).collect::<String>())
    } else {
        text
    }
}

async fn fetch_grid(
    service: &str,
    url: &str,
    area: (f64, f64, f64, f64),
) -> AppResult<raster::Grid> {
    let res = get_with_retry(service, url).await?;
    let status = res.status();
    let bytes = res.bytes().await.map_err(|e| upstream(service, e))?;
    if !status.is_success() {
        let text = summarize_body(&bytes);
        return Err(AppError::Upstream(format!(
            "{service} returned HTTP {status}{}",
            if text.is_empty() {
                String::new()
            } else {
                format!(": {text}")
            }
        )));
    }
    let grid = raster::read_grid(&bytes, area).map_err(|e| {
        let head = summarize_body(&bytes[..bytes.len().min(120)]);
        AppError::Upstream(format!("{service}: {e} (response starts with {head:?})"))
    })?;
    let covered = raster::coverage(&grid, area);
    if covered < 0.5 {
        return Err(AppError::Upstream(format!(
            "{service} returned data for a different area (covers {:.0}% of the request; \
             check the subset axis order in the request template)",
            covered * 100.0
        )));
    }
    Ok(grid)
}

/// Side of one GUGiK tile, metres. NMPT is a 0.5 m ASCII grid that GUGiK
/// renders slowly (about a minute per 0.03 km²), so tiles are small and only
/// those near the track are fetched.
pub const GUGIK_TILE_M: f64 = 150.0;
/// Tiles within this distance of the track are fetched.
pub const CORRIDOR_M: f64 = 50.0;
/// Upper bound of tiles per flight (~1 min each); nearest to take-off first.
pub const MAX_TILES: usize = 12;

/// Progress of a running LiDAR fetch, shared with the API.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

pub struct LidarRequest<'a> {
    pub dtm_template: &'a str,
    pub dsm_template: &'a str,
    /// Track points (lon, lat).
    pub track: &'a [(f64, f64)],
    /// Take-off point (lon, lat): its ground level is the height reference.
    pub reference: (f64, f64),
    pub progress: Arc<Progress>,
}

/// Tiles of `size` m (aligned to multiples of `size` in EPSG:2180) that lie
/// within `margin` of any track point, nearest to `reference` first, at most
/// `max`. Returns the tiles and whether the list was cut.
pub fn corridor_tiles(
    points: &[(f64, f64)],
    reference: (f64, f64),
    size: f64,
    margin: f64,
    max: usize,
) -> (Vec<(f64, f64, f64, f64)>, bool) {
    let mut keys: Vec<(i64, i64)> = Vec::new();
    for &(e, n) in points {
        let (i0, i1) = (
            ((e - margin) / size).floor() as i64,
            ((e + margin) / size).floor() as i64,
        );
        let (j0, j1) = (
            ((n - margin) / size).floor() as i64,
            ((n + margin) / size).floor() as i64,
        );
        for i in i0..=i1 {
            for j in j0..=j1 {
                keys.push((i, j));
            }
        }
    }
    keys.sort_unstable();
    keys.dedup();
    let centre = |&(i, j): &(i64, i64)| {
        let (ce, cn) = ((i as f64 + 0.5) * size, (j as f64 + 0.5) * size);
        (ce - reference.0).powi(2) + (cn - reference.1).powi(2)
    };
    keys.sort_by(|a, b| centre(a).total_cmp(&centre(b)));
    let truncated = keys.len() > max;
    keys.truncate(max);
    let tiles = keys
        .into_iter()
        .map(|(i, j)| {
            let (e, n) = (i as f64 * size, j as f64 * size);
            (e, n, e + size, n + size)
        })
        .collect();
    (tiles, truncated)
}

/// Object heights (NMPT − NMT) from GUGiK for the flight's corridor, as
/// GeoJSON polygons with `height` (above local ground) and `groundRel`
/// (ground relative to take-off). `truncated` tells whether the corridor was
/// longer than `MAX_TILES` allows.
pub async fn gugik_lidar(req: LidarRequest<'_>) -> AppResult<Value> {
    if !req
        .track
        .iter()
        .all(|&(lon, lat)| puwg92::in_poland(lon, lat))
    {
        return Err(AppError::NotAvailable(
            "LiDAR heights from GUGiK are only available for flights in Poland.".into(),
        ));
    }
    let points: Vec<(f64, f64)> = req
        .track
        .iter()
        .map(|&(lon, lat)| puwg92::forward(lon, lat))
        .collect();
    let reference = puwg92::forward(req.reference.0, req.reference.1);
    let (tiles, truncated) =
        corridor_tiles(&points, reference, GUGIK_TILE_M, CORRIDOR_M, MAX_TILES);
    req.progress.total.store(tiles.len(), Ordering::SeqCst);

    // One request at a time: GUGiK drops connections intermittently and is
    // slow to render NMPT, and parallel connections make both worse.
    let mut pairs = Vec::with_capacity(tiles.len());
    for (k, tile) in tiles.iter().enumerate() {
        let label = |service: &str| format!("{service} (tile {}/{})", k + 1, tiles.len());
        let dtm = fetch_grid(
            &label("GUGiK NMT"),
            &fill_template(req.dtm_template, *tile),
            *tile,
        )
        .await?;
        let dsm = fetch_grid(
            &label("GUGiK NMPT"),
            &fill_template(req.dsm_template, *tile),
            *tile,
        )
        .await?;
        pairs.push((*tile, dtm, dsm));
        req.progress.done.store(k + 1, Ordering::SeqCst);
    }

    let dtm_all = raster::Mosaic(pairs.iter().map(|(_, d, _)| d.clone()).collect());
    let reference_ground = dtm_all
        .at(reference.0, reference.1)
        .map(f64::from)
        .unwrap_or_else(|| {
            // Take-off outside the fetched tiles: use the lowest ground instead.
            dtm_all
                .0
                .iter()
                .flat_map(|g| g.data.iter().copied())
                .filter(|v| v.is_finite() && *v > -1000.0)
                .fold(f32::MAX, f32::min) as f64
        });

    let blocks = tokio::task::spawn_blocking(move || {
        let params = BlockParams {
            cell_m: 1.0,
            min_height_m: 2.5,
            reference_ground,
            max_blocks: 60_000,
        };
        pairs
            .iter()
            .flat_map(|(tile, dtm, dsm)| raster::blocks(dsm, dtm, *tile, &params))
            .take(params.max_blocks)
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let features: Vec<Value> = blocks
        .into_iter()
        .map(|blk| {
            let ring: Vec<[f64; 2]> = [
                (blk.west, blk.south),
                (blk.east, blk.south),
                (blk.east, blk.north),
                (blk.west, blk.north),
                (blk.west, blk.south),
            ]
            .iter()
            .map(|&(e, n)| {
                let (lon, lat) = puwg92::inverse(e, n);
                [(lon * 1e7).round() / 1e7, (lat * 1e7).round() / 1e7]
            })
            .collect();
            json!({
                "type": "Feature",
                "properties": { "height": blk.height, "groundRel": blk.ground_rel },
                "geometry": { "type": "Polygon", "coordinates": [ring] },
            })
        })
        .collect();
    Ok(json!({
        "type": "FeatureCollection",
        "features": features,
        "tiles": tiles.len(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(parse_length_m("12"), Some(12.0));
        assert_eq!(parse_length_m("12.5 m"), Some(12.5));
        assert_eq!(parse_length_m("7,5m"), Some(7.5));
        assert!((parse_length_m("40 ft").unwrap() - 12.192).abs() < 1e-9);
        assert_eq!(parse_length_m("tall"), None);
        assert_eq!(parse_length_m("0"), None);
        assert_eq!(parse_length_m("900"), None);
    }

    #[test]
    fn bbox_margin_and_clamp() {
        let b = BBox::around(&[(21.0, 52.0), (21.01, 52.005)], 100.0).unwrap();
        assert!(b.west < 21.0 && b.east > 21.01 && b.south < 52.0 && b.north > 52.005);
        let small = b.clamp_size(200.0);
        let w_m = (small.east - small.west) * 111_320.0 * 52.0f64.to_radians().cos();
        assert!((w_m - 200.0).abs() < 1.0, "{w_m}");
        assert!(BBox::around(&[], 10.0).is_none());
    }

    #[test]
    fn error_bodies_are_summarized() {
        let html = br#"<!DOCTYPE HTML PUBLIC "-//IETF//DTD HTML 2.0//EN"> <html><head> <title>404 Not Found</title> </head><body> <h1>Not Found</h1> <p>The requested URL was not found on this server.</p></body></html>"#;
        assert_eq!(
            summarize_body(html),
            "404 Not Found Not Found The requested URL was not found on this server."
        );
        assert_eq!(summarize_body(b""), "");
    }

    #[test]
    fn corridor_tiles_follow_the_track() {
        // A 400 m track going east from (1000, 975).
        let pts: Vec<(f64, f64)> = (0..=40)
            .map(|i| (1000.0 + i as f64 * 10.0, 975.0))
            .collect();
        let (tiles, truncated) = corridor_tiles(&pts, (1000.0, 975.0), 150.0, 50.0, 100);
        assert!(!truncated);
        // 975 ± 50 stays inside the 900..1050 row; 950..1450 spans 4 columns.
        assert!(
            tiles.iter().all(|t| t.1 == 900.0 && t.3 == 1050.0),
            "{tiles:?}"
        );
        assert_eq!(tiles.len(), 4);
        assert_eq!(
            tiles[0],
            (900.0, 900.0, 1050.0, 1050.0),
            "nearest to take-off first"
        );
        assert_eq!(tiles[3], (1350.0, 900.0, 1500.0, 1050.0));
        let (cut, truncated) = corridor_tiles(&pts, (1000.0, 975.0), 150.0, 50.0, 2);
        assert!(truncated);
        assert_eq!(cut.len(), 2);
    }

    #[test]
    fn template_placeholders() {
        let url = fill_template(
            "x?SUBSET=x({minN},{maxN})&SUBSET=y({minE},{maxE})",
            (1.0, 2.0, 3.0, 4.0),
        );
        assert_eq!(url, "x?SUBSET=x(2.0,4.0)&SUBSET=y(1.0,3.0)");
    }
}
