//! Upload pipeline: bytes -> (keychains) -> frames -> flight model -> SQLite.

use dji_log_parser::DJILog;
use dji_log_parser::keychain::KeychainsRequest;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::db;
use crate::error::{AppError, AppResult};
use crate::flight::{ParsedFlight, build_flight};
use crate::keychain::{self, Keychains};

/// Highest log format version `dji-log-parser` 0.5.7 understands.
pub const MAX_SUPPORTED_VERSION: u8 = 14;
/// First version whose records are AES-encrypted with keys from the DJI API.
pub const FIRST_ENCRYPTED_VERSION: u8 = 13;

/// Version of the parse/mapping code. Bump it when `flight.rs` starts
/// extracting new data: re-uploading a file then refreshes flights stored by
/// an older version instead of reporting a plain duplicate.
pub const PARSE_VERSION: i64 = 2;

#[derive(Debug)]
pub struct IngestOutcome {
    pub id: i64,
    /// `false` when the identical file had already been imported.
    pub created: bool,
    /// An existing flight was re-parsed with the current `PARSE_VERSION`.
    pub reparsed: bool,
}

pub async fn ingest(state: &AppState, file_name: &str, bytes: Vec<u8>) -> AppResult<IngestOutcome> {
    if bytes.is_empty() {
        return Err(AppError::BadRequest("The uploaded file is empty.".into()));
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if let Some((id, version)) = db::find_by_hash(&state.pool, &sha256).await? {
        let mut outcome = IngestOutcome {
            id,
            created: false,
            reparsed: false,
        };
        if version < PARSE_VERSION {
            // Best effort: if parsing fails now (e.g. the API key was removed),
            // keep the data that is already stored.
            match parse(state, bytes).await {
                Ok(flight) => {
                    db::replace_flight(&state.pool, id, PARSE_VERSION, &flight).await?;
                    tracing::info!(id, from = version, to = PARSE_VERSION, "re-parsed flight");
                    outcome.reparsed = true;
                }
                Err(e) => tracing::warn!(id, code = e.code(), "re-parse failed, keeping old data"),
            }
        }
        return Ok(outcome);
    }

    let size = bytes.len();
    let flight = parse(state, bytes).await?;

    match db::insert_flight(
        &state.pool,
        file_name,
        &sha256,
        size,
        PARSE_VERSION,
        &flight,
    )
    .await
    {
        Ok(id) => {
            tracing::info!(
                id,
                version = flight.meta.log_version,
                samples = flight.meta.sample_count,
                "imported flight log"
            );
            Ok(IngestOutcome {
                id,
                created: true,
                reparsed: false,
            })
        }
        // Lost a race against a concurrent upload of the same file.
        Err(e) => match db::find_by_hash(&state.pool, &sha256).await? {
            Some((id, _)) => Ok(IngestOutcome {
                id,
                created: false,
                reparsed: false,
            }),
            None => Err(e),
        },
    }
}

/// Parses a log into the flight model, fetching (or reusing cached) keychains
/// for encrypted logs.
pub async fn parse(state: &AppState, bytes: Vec<u8>) -> AppResult<ParsedFlight> {
    check_version(peek_version(&bytes)?)?;
    let (log, request) = run_parser(move || {
        let log = DJILog::from_bytes(bytes).map_err(|e| AppError::InvalidLog(e.to_string()))?;
        check_version(log.version)?;
        let request = if log.version >= FIRST_ENCRYPTED_VERSION {
            Some(
                log.keychains_request()
                    .map_err(|e| AppError::InvalidLog(e.to_string()))?,
            )
        } else {
            None
        };
        Ok((log, request))
    })
    .await?;

    let version = log.version;
    let keychains = match &request {
        Some(req) => Some(resolve_keychains(state, version, req).await?),
        None => None,
    };

    let result = run_parser(move || {
        let frames = log
            .frames(keychains)
            .map_err(|e| AppError::InvalidLog(e.to_string()))?;
        Ok(build_flight(log.version, &log.details, &frames))
    })
    .await;

    let flight = match result {
        // The parser panics on malformed keys; for encrypted logs that means bad keys.
        Err(AppError::InvalidLog(_)) if request.is_some() => Err(AppError::DecryptionFailed),
        other => other,
    }
    .and_then(validate);

    if let (Err(AppError::DecryptionFailed), Some(req)) = (&flight, &request) {
        // Never keep serving keys that did not work.
        if let Ok(hash) = keychain::request_hash(req) {
            let _ = db::delete_keychains(&state.pool, &hash).await;
        }
    }
    flight
}

/// Reads the format version from the 100-byte file prefix without parsing
/// the rest, so unsupported files get a precise error.
pub fn peek_version(bytes: &[u8]) -> AppResult<u8> {
    const PREFIX_SIZE: usize = 100;
    const VERSION_OFFSET: usize = 10;
    if bytes.len() < PREFIX_SIZE {
        return Err(AppError::InvalidLog("the file is too short".into()));
    }
    Ok(bytes[VERSION_OFFSET])
}

pub fn check_version(version: u8) -> AppResult<()> {
    if version == 0 || version > MAX_SUPPORTED_VERSION {
        return Err(AppError::UnsupportedVersion(version));
    }
    Ok(())
}

fn validate(flight: ParsedFlight) -> AppResult<ParsedFlight> {
    if flight.samples.is_empty() {
        if flight.meta.encrypted {
            return Err(AppError::DecryptionFailed);
        }
        return Err(AppError::InvalidLog(
            "the log contains no telemetry records".into(),
        ));
    }
    if flight.meta.encrypted {
        // Records decrypted with wrong keys come out as zeros.
        let any_signal = flight
            .samples
            .iter()
            .any(|s| s.lat.is_some() || s.height_m != 0.0 || s.gps_sats != 0);
        if !any_signal {
            return Err(AppError::DecryptionFailed);
        }
    }
    Ok(flight)
}

async fn resolve_keychains(
    state: &AppState,
    version: u8,
    request: &KeychainsRequest,
) -> AppResult<Keychains> {
    let hash = keychain::request_hash(request)?;
    if let Some(cached) = db::cached_keychains(&state.pool, &hash).await? {
        tracing::debug!("using cached keychains");
        return Ok(cached);
    }
    let api_key = state
        .config
        .api_key
        .as_ref()
        .ok_or(AppError::MissingApiKey(version))?;
    let keychains = keychain::fetch(
        &state.http,
        &state.config.keychain_endpoint,
        api_key,
        request,
    )
    .await?;
    db::store_keychains(&state.pool, &hash, &keychains).await?;
    tracing::info!("fetched keychains from DJI API");
    Ok(keychains)
}

/// Runs CPU-bound parser code off the async runtime and turns parser panics
/// (the crate `unwrap`s on some corrupt inputs) into errors.
async fn run_parser<T, F>(f: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(e) if e.is_panic() => Err(AppError::InvalidLog(
            "the parser could not read this file (corrupted or truncated log?)".into(),
        )),
        Err(e) => Err(AppError::Internal(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_bounds() {
        assert!(check_version(1).is_ok());
        assert!(check_version(14).is_ok());
        assert_eq!(check_version(0).unwrap_err().code(), "unsupported_version");
        assert_eq!(check_version(15).unwrap_err().code(), "unsupported_version");
    }
}
