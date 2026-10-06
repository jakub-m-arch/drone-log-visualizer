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
