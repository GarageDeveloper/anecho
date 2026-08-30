//! Client for REW's REST API (REW >= 5.20 started with API enabled; default port 4735).
//!
//! Only the endpoints the bench needs are typed. REW serves its own Swagger 2.0 spec at
//! `http://localhost:4735/swagger-spec.js` (378 paths in REW 5.40 / API 0.9.6).

use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

pub const DEFAULT_BASE_URL: &str = "http://localhost:4735";

#[derive(Debug, Error)]
pub enum RewError {
    #[error("REW is not reachable at {url} (start REW with its API enabled): {source}")]
    Unreachable { url: String, source: reqwest::Error },
    #[error("REW request failed: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    /// e.g. "5.40 Beta 133 API 0.9.6"
    pub message: String,
}

impl Version {
    /// ("5.40 Beta 133", "0.9.6") when the message follows REW's format.
    pub fn split(&self) -> (String, String) {
        match self.message.split_once(" API ") {
            Some((app, api)) => (app.trim().into(), api.trim().into()),
            None => (self.message.clone(), String::new()),
        }
    }
}

/// One entry of REW's measurement list (`GET /measurements`), subset of fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub title: String,
    pub uuid: String,
    pub date: String,
    pub sample_rate: f64,
    pub start_freq: f64,
    pub end_freq: f64,
}

#[derive(Debug, Clone)]
pub struct Rew {
    base: String,
    http: reqwest::Client,
}

impl Rew {
    pub fn new(base_url: &str) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, RewError> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.get(&url).send().await.map_err(|source| {
            if source.is_connect() {
                RewError::Unreachable {
                    url: url.clone(),
                    source,
                }
            } else {
                RewError::Http(source)
            }
        })?;
        Ok(resp.error_for_status()?.json::<T>().await?)
    }

    pub async fn version(&self) -> Result<Version, RewError> {
        self.get("/version").await
    }

    /// Input devices of the Java audio driver (names as REW shows them).
    pub async fn input_devices(&self) -> Result<Vec<String>, RewError> {
        self.get("/audio/java/input-devices").await
    }

    pub async fn output_devices(&self) -> Result<Vec<String>, RewError> {
        self.get("/audio/java/output-devices").await
    }

    pub async fn sample_rate(&self) -> Result<serde_json::Value, RewError> {
        self.get("/audio/samplerate").await
    }

    /// Measurements currently loaded in REW, keyed by their index.
    pub async fn measurements(&self) -> Result<BTreeMap<String, Measurement>, RewError> {
        self.get("/measurements").await
    }
}
