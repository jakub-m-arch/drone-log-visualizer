use std::fmt;
use std::path::PathBuf;

/// DJI Open API key. Wrapped so it can never end up in logs via `Debug`.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Option<Self> {
        let key = key.into().trim().to_string();
        (!key.is_empty()).then_some(ApiKey(key))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

pub const DEFAULT_TERRAIN_URL: &str =
    "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";

/// OpenFreeMap: free OpenMapTiles-schema vector tiles, no key.
pub const DEFAULT_VECTOR_TILES_URL: &str = "https://tiles.openfreemap.org/planet";

pub const DEFAULT_OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";

/// GUGiK WCS 2.0 GetCoverage requests for the 1 m NMT and NMPT grids, both
/// with heights in the Kronsztadt 86 datum so their difference is consistent.
/// Subset axes in EPSG:2180: `x` = northing, `y` = easting.
pub const DEFAULT_GUGIK_NMT_URL: &str = "https://mapy.geoportal.gov.pl/wss/service/PZGIK/NMT/GRID1/WCS/DigitalTerrainModelFormatTIFF?SERVICE=WCS&VERSION=2.0.1&REQUEST=GetCoverage&COVERAGEID=DTM_PL-KRON86-NH_TIFF&FORMAT=image/tiff&SUBSET=x({minN},{maxN})&SUBSET=y({minE},{maxE})";
pub const DEFAULT_GUGIK_NMPT_URL: &str = "https://mapy.geoportal.gov.pl/wss/service/PZGIK/NMPT/GRID1/WCS/DigitalSurfaceModel?SERVICE=WCS&VERSION=2.0.1&REQUEST=GetCoverage&COVERAGEID=DSM_PL-KRON86-NH&FORMAT=image/tiff&SUBSET=x({minN},{maxN})&SUBSET=y({minE},{maxE})";

/// `off` disables, a value overrides, nothing selects the default.
fn optional_url(value: Option<String>, default: &str) -> Option<String> {
    match value {
        Some(v) if v.eq_ignore_ascii_case("off") => None,
        Some(v) => Some(v),
        None => Some(default.into()),
    }
}

pub const DEFAULT_KEYCHAIN_ENDPOINT: &str =
    "https://dev.dji.com/openapi/v1/flight-records/keychains";

#[derive(Debug, Clone)]
pub struct Config {
    pub api_key: Option<ApiKey>,
    pub data_dir: PathBuf,
    pub static_dir: PathBuf,
    pub port: u16,
    pub max_upload_bytes: usize,
    pub keychain_endpoint: String,
    pub map_tile_url: String,
    pub map_attribution: String,
    /// Raster DEM tiles for the 3D view; `None` = flat 3D without terrain.
    pub terrain_url: Option<String>,
    /// `terrarium` or `mapbox` (Terrain-RGB) encoding of `terrain_url`.
    pub terrain_encoding: String,
    pub terrain_attribution: String,
    /// TileJSON URL of OpenMapTiles-schema vector tiles used for 3D buildings
    /// and woods; `None` disables them.
    pub vector_tiles_url: Option<String>,
    /// Overpass API endpoint for OSM trees; `None` disables the layer.
    pub overpass_url: Option<String>,
    /// GUGiK NMT (terrain) and NMPT (surface) GeoTIFF request templates with
    /// `{minE} {minN} {maxE} {maxN}` in EPSG:2180; `None` disables LiDAR.
    pub gugik_nmt_url: Option<String>,
    pub gugik_nmpt_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let port = match var("PORT") {
            Some(p) => p
                .parse()
                .map_err(|_| format!("PORT must be a number, got {p:?}"))?,
            None => 8080,
        };
        let max_upload_mb: usize = match var("MAX_UPLOAD_MB") {
            Some(v) => v
                .parse()
                .map_err(|_| format!("MAX_UPLOAD_MB must be a number, got {v:?}"))?,
            None => 200,
        };
        Ok(Config {
            api_key: var("DJI_API_KEY").and_then(ApiKey::new),
            data_dir: var("DATA_DIR").unwrap_or_else(|| "/data".into()).into(),
            static_dir: var("STATIC_DIR").unwrap_or_else(|| "./static".into()).into(),
            port,
            max_upload_bytes: max_upload_mb * 1024 * 1024,
            keychain_endpoint: var("DJI_KEYCHAIN_ENDPOINT")
                .unwrap_or_else(|| DEFAULT_KEYCHAIN_ENDPOINT.into()),
            map_tile_url: var("MAP_TILE_URL")
                .unwrap_or_else(|| "https://tile.openstreetmap.org/{z}/{x}/{y}.png".into()),
            // Public AWS Open Data terrain tiles; set MAP_TERRAIN_URL=off to disable.
            terrain_url: match var("MAP_TERRAIN_URL") {
                Some(v) if v.eq_ignore_ascii_case("off") => None,
                Some(v) => Some(v),
                None => Some(DEFAULT_TERRAIN_URL.into()),
            },
            terrain_encoding: match var("MAP_TERRAIN_ENCODING") {
                Some(v) if v == "terrarium" || v == "mapbox" => v,
                Some(v) => {
                    return Err(format!(
                        "MAP_TERRAIN_ENCODING must be terrarium or mapbox, got {v:?}"
                    ));
                }
                None => "terrarium".into(),
            },
            overpass_url: optional_url(var("OVERPASS_URL"), DEFAULT_OVERPASS_URL),
            gugik_nmt_url: optional_url(var("GUGIK_NMT_URL"), DEFAULT_GUGIK_NMT_URL),
            gugik_nmpt_url: optional_url(var("GUGIK_NMPT_URL"), DEFAULT_GUGIK_NMPT_URL),
            vector_tiles_url: match var("MAP_VECTOR_TILES_URL") {
                Some(v) if v.eq_ignore_ascii_case("off") => None,
                Some(v) => Some(v),
                None => Some(DEFAULT_VECTOR_TILES_URL.into()),
            },
            terrain_attribution: var("MAP_TERRAIN_ATTRIBUTION").unwrap_or_else(|| {
                "Elevation: <a href=\"https://github.com/tilezen/joerd/blob/master/docs/attribution.md\">Mapzen Terrain Tiles</a>".into()
            }),
            map_attribution: var("MAP_ATTRIBUTION").unwrap_or_else(|| {
                "© <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors"
                    .into()
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_is_redacted_in_debug_output() {
        let key = ApiKey::new("super-secret-123").unwrap();
        let cfg = format!("{:?}", Some(key.clone()));
        assert!(!cfg.contains("super-secret"), "{cfg}");
        assert_eq!(key.expose(), "super-secret-123");
    }

    #[test]
    fn blank_api_key_is_treated_as_missing() {
        assert!(ApiKey::new("   ").is_none());
    }
}
