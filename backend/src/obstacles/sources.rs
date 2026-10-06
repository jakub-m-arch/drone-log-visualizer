//! Fetching obstacle data from external services.

use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

use super::puwg92;
use super::raster::{self, BlockParams};
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
    reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
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
        match client().get(url).send().await {
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
pub async fn osm_trees(endpoint: &str, b: BBox) -> AppResult<Value> {
    let query = format!(
        "[out:json][timeout:60];node[\"natural\"=\"tree\"]({},{},{},{});out body 20000;",
        b.south, b.west, b.north, b.east
    );
    let res = client()
        .post(endpoint)
        .form(&[("data", query)])
        .send()
        .await
        .map_err(|e| upstream("Overpass API", e))?;
    if !res.status().is_success() {
        return Err(AppError::Upstream(format!(
            "Overpass API returned HTTP {} (it may be busy, try again later)",
            res.status()
        )));
    }
    let body: OverpassResponse = res
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("Overpass API: unexpected response: {e}")))?;
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
    raster::read_grid(&bytes, area).map_err(|e| {
        let head = summarize_body(&bytes[..bytes.len().min(120)]);
        AppError::Upstream(format!("{service}: {e} (response starts with {head:?})"))
    })
}

pub struct LidarRequest<'a> {
    pub dtm_template: &'a str,
    pub dsm_template: &'a str,
    pub bbox: BBox,
    /// Take-off point (lon, lat): its ground level is the height reference.
    pub reference: (f64, f64),
}

/// Object heights (NMPT − NMT) from GUGiK as GeoJSON polygons with
/// `height` (above local ground) and `groundRel` (ground relative to take-off).
pub async fn gugik_lidar(req: LidarRequest<'_>) -> AppResult<Value> {
    let b = req.bbox;
    for (lon, lat) in [(b.west, b.south), (b.east, b.north)] {
        if !puwg92::in_poland(lon, lat) {
            return Err(AppError::NotAvailable(
                "LiDAR heights from GUGiK are only available for flights in Poland.".into(),
            ));
        }
    }
    // Projected box covering the geographic one.
    let corners = [
        puwg92::forward(b.west, b.south),
        puwg92::forward(b.west, b.north),
        puwg92::forward(b.east, b.south),
        puwg92::forward(b.east, b.north),
    ];
    let min_e = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor();
    let max_e = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil();
    let min_n = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor();
    let max_n = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil();
    let area = (min_e, min_n, max_e, max_n);

    let (dtm_url, dsm_url) = (
        fill_template(req.dtm_template, area),
        fill_template(req.dsm_template, area),
    );
    // One request at a time: GUGiK drops connections intermittently, and
    // parallel connections from one address make it worse. Both requests are
    // still made so an error names every failing service.
    let dtm = fetch_grid("GUGiK NMT", &dtm_url, area).await;
    let dsm = fetch_grid("GUGiK NMPT", &dsm_url, area).await;
    let (dtm, dsm) = match (dtm, dsm) {
        (Ok(dtm), Ok(dsm)) => (dtm, dsm),
        (Err(e), Ok(_)) | (Ok(_), Err(e)) => return Err(e),
        (Err(a), Err(b)) => return Err(AppError::Upstream(format!("{a}; {b}"))),
    };

    let (re, rn) = puwg92::forward(req.reference.0, req.reference.1);
    let reference_ground = dtm.at(re, rn).map(f64::from).unwrap_or_else(|| {
        // Take-off outside the returned grid: use the lowest ground instead.
        dtm.data
            .iter()
            .copied()
            .filter(|v| v.is_finite() && *v > -1000.0)
            .fold(f32::MAX, f32::min) as f64
    });

    let blocks = tokio::task::spawn_blocking(move || {
        raster::blocks(
            &dsm,
            &dtm,
            (min_e, min_n, max_e, max_n),
            &BlockParams {
                cell_m: 1.0,
                min_height_m: 2.5,
                reference_ground,
                max_blocks: 60_000,
            },
        )
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
    Ok(json!({ "type": "FeatureCollection", "features": features }))
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
    fn template_placeholders() {
        let url = fill_template(
            "x?SUBSET=x({minN},{maxN})&SUBSET=y({minE},{maxE})",
            (1.0, 2.0, 3.0, 4.0),
        );
        assert_eq!(url, "x?SUBSET=x(2.0,4.0)&SUBSET=y(1.0,3.0)");
    }
}
