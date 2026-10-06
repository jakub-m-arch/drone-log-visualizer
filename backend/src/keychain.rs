//! Fetching AES keychains for encrypted (v13+) logs from the DJI Open API.
//!
//! `dji-log-parser` ships a blocking `ureq` client for this. We only use the
//! crate to build the request body and call the endpoint ourselves with
//! `reqwest`, which gives us async I/O, proxy support and precise error mapping.

use dji_log_parser::keychain::{KeychainFeaturePoint, KeychainsRequest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::config::ApiKey;
use crate::error::{AppError, AppResult};

pub type Keychains = Vec<Vec<KeychainFeaturePoint>>;

#[derive(Debug, Deserialize)]
struct KeychainsResponse {
    data: Option<Keychains>,
    result: Option<KeychainsResult>,
}

#[derive(Debug, Deserialize)]
struct KeychainsResult {
    code: i64,
    #[serde(default)]
    msg: String,
}

/// Stable cache key for a keychain request.
pub fn request_hash(request: &KeychainsRequest) -> AppResult<String> {
    let body = serde_json::to_vec(request).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(hex::encode(Sha256::digest(&body)))
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("dji-log-viewer/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client")
}

pub async fn fetch(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &ApiKey,
    request: &KeychainsRequest,
) -> AppResult<Keychains> {
    let response = client
        .post(endpoint)
        .header("Api-Key", api_key.expose())
        .json(request)
        .send()
        .await
        .map_err(network_error)?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AppError::InvalidApiKey);
    }
    let body = response.text().await.map_err(network_error)?;
    if !status.is_success() {
        return Err(AppError::DjiApi(format!(
            "HTTP {status}: {}",
            truncate(&body, 200)
        )));
    }
    parse_response(&body)
}

fn network_error(e: reqwest::Error) -> AppError {
    AppError::Network(format!(
        "the DJI API: {}",
        crate::error::describe_request_error(e)
    ))
}

fn parse_response(body: &str) -> AppResult<Keychains> {
    let parsed: KeychainsResponse = serde_json::from_str(body)
        .map_err(|_| AppError::DjiApi(format!("unexpected response: {}", truncate(body, 200))))?;
    if let Some(result) = &parsed.result
        && result.code != 0
    {
        let msg = if result.msg.is_empty() {
            format!("error code {}", result.code)
        } else {
            format!("{} (code {})", result.msg, result.code)
        };
        return Err(AppError::DjiApi(msg));
    }
    parsed
        .data
        .ok_or_else(|| AppError::DjiApi("response contained no keychain data".into()))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_successful_response() {
        let body = r#"{"result":{"code":0,"msg":"success"},"data":[[{"featurePoint":"FR_Standardization_Feature_Base_1","aesKey":"a2V5","aesIv":"aXY="}]]}"#;
        let kc = parse_response(body).unwrap();
        assert_eq!(kc.len(), 1);
        assert_eq!(kc[0][0].aes_key, "a2V5");
    }

    #[test]
    fn maps_api_error_code() {
        let body = r#"{"result":{"code":3,"msg":"invalid params"},"data":null}"#;
        let err = parse_response(body).unwrap_err();
        assert_eq!(err.code(), "dji_api_error");
        assert!(err.to_string().contains("invalid params"));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(
            parse_response("<html>").unwrap_err().code(),
            "dji_api_error"
        );
    }
}
