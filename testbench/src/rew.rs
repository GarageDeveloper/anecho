//! Client for REW's REST API (REW >= 5.20 started with API enabled; default port 4735).
//!
//! Only the endpoints the bench needs are typed. REW serves its own Swagger 2.0 spec at
//! `http://localhost:4735/swagger-spec.js` (378 paths in REW 5.40 / API 0.9.6).

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const DEFAULT_BASE_URL: &str = "http://localhost:4735";

#[derive(Debug, Error)]
pub enum RewError {
    #[error("REW is not reachable at {url} (start REW with its API enabled): {source}")]
    Unreachable { url: String, source: reqwest::Error },
    #[error("REW request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("REW returned no data: {0}")]
    NoData(String),
    #[error("REW payload could not be decoded: {0}")]
    Decode(String),
}

/// `{"value": x, "unit": "..."}` — REW's generic quantity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Value {
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Command<'a> {
    command: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    parameters: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiResponse {
    pub message: String,
    #[serde(default)]
    pub valid_values: Vec<String>,
}

/// `GET/POST /rta/configuration` (subset; unknown fields are preserved by REW).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtaConfiguration {
    /// "Spectrum" or "RTA 1/3 octave" etc. (`/rta/configuration/modes`).
    pub mode: String,
    pub smoothing: String,
    /// "8k" ... "4M" (`/rta/configuration/fft-lengths`).
    pub fft_length: String,
    /// "Rectangular", "Hann", "Blackman-Harris 4", "Blackman-Harris 7", "Flat-Top", ...
    pub window: String,
    /// "None", "2", "4", ..., "Exponential 0.50", ..., "Forever".
    pub averaging: String,
    #[serde(default)]
    pub calc_distortion_enabled: bool,
    #[serde(default)]
    pub fundamental_from_sine_gen: bool,
    #[serde(default)]
    pub use64_bit_fft: bool,
    #[serde(default)]
    pub adjust_rta_levels: bool,
    #[serde(default)]
    pub stop_at: bool,
    #[serde(default)]
    pub stop_at_value: i32,
    #[serde(default)]
    pub maximum_overlap: String,
    #[serde(default)]
    pub restart_capture_on_generator_change: bool,
    #[serde(default)]
    pub stop_generator_with_rta: bool,
}

/// Decoded `GET /rta/captured-data`: a linear-frequency spectrum.
#[derive(Debug, Clone)]
pub struct Spectrum {
    pub unit: String,
    pub start_hz: f64,
    pub step_hz: f64,
    pub magnitude: Vec<f32>,
    pub total_samples_processed: u64,
}

impl Spectrum {
    pub fn frequency(&self, bin: usize) -> f64 {
        self.start_hz + bin as f64 * self.step_hz
    }
    /// Bin closest to `hz`.
    pub fn bin_at(&self, hz: f64) -> usize {
        (((hz - self.start_hz) / self.step_hz).round().max(0.0) as usize)
            .min(self.magnitude.len().saturating_sub(1))
    }
    /// (frequency, level) of the highest bin.
    pub fn peak(&self) -> Option<(f64, f32)> {
        let (i, v) = self.magnitude.iter().copied().enumerate().fold(
            None,
            |m: Option<(usize, f32)>, (i, v)| match m {
                Some((_, mv)) if mv >= v => m,
                _ => Some((i, v)),
            },
        )?;
        Some((self.frequency(i), v))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrequencyResponseWire {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    start_freq: f64,
    #[serde(default)]
    freq_step: f64,
    #[serde(default)]
    magnitude: String,
    #[serde(default)]
    total_samples_processed: u64,
}

/// `GET /rta/distortion` (subset).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtaDistortion {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub fundamental_frequency: f64,
    #[serde(default, rename = "fundamentaldBFS")]
    pub fundamental_dbfs: f64,
    pub fundamental_level: Option<Value>,
    pub thd: Option<Value>,
    pub thd_plus_n: Option<Value>,
    #[serde(default)]
    pub harmonics: Vec<Value>,
    pub imd: Option<Value>,
    #[serde(default)]
    pub averages: i32,
    #[serde(default)]
    pub snrd_b: f64,
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

    async fn post<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, RewError> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.post(&url).json(body).send().await?;
        Ok(resp.error_for_status()?.json::<T>().await?)
    }

    async fn post_text<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<String, RewError> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.post(&url).json(body).send().await?;
        Ok(resp.error_for_status()?.text().await?)
    }

    // ---- audio device selection (Java driver) ----

    pub async fn input_device_name(&self) -> Result<String, RewError> {
        let v: serde_json::Value = self.get("/audio/java/input-device").await?;
        Ok(v["device"].as_str().unwrap_or_default().to_string())
    }

    pub async fn output_device_name(&self) -> Result<String, RewError> {
        let v: serde_json::Value = self.get("/audio/java/output-device").await?;
        Ok(v["device"].as_str().unwrap_or_default().to_string())
    }

    pub async fn set_input_device(&self, name: &str) -> Result<(), RewError> {
        self.post_text(
            "/audio/java/input-device",
            &serde_json::json!({ "device": name }),
        )
        .await
        .map(|_| ())
    }

    pub async fn set_output_device(&self, name: &str) -> Result<(), RewError> {
        self.post_text(
            "/audio/java/output-device",
            &serde_json::json!({ "device": name }),
        )
        .await
        .map(|_| ())
    }

    pub async fn set_sample_rate(&self, hz: f64) -> Result<(), RewError> {
        self.post_text(
            "/audio/samplerate",
            &Value {
                value: hz,
                unit: "Hz".into(),
            },
        )
        .await
        .map(|_| ())
    }

    // ---- generator ----

    /// `signal` is one of `/generator/signals` ("sine", "dualtone", "pinknoise", ...).
    pub async fn set_generator_signal(&self, signal: &str) -> Result<(), RewError> {
        self.post_text(
            "/generator/signal",
            &serde_json::json!({ "signal": signal }),
        )
        .await
        .map(|_| ())
    }

    /// Level in one of `/generator/level/units`: "dBFS", "dBV", "dBu", "V".
    pub async fn set_generator_level(&self, value: f64, unit: &str) -> Result<(), RewError> {
        self.post_text(
            "/generator/level",
            &Value {
                value,
                unit: unit.into(),
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn set_sine_frequency(&self, hz: f64) -> Result<(), RewError> {
        self.post_text(
            "/generator/frequency",
            &Value {
                value: hz,
                unit: "Hz".into(),
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn generator_command(&self, command: &str) -> Result<ApiResponse, RewError> {
        self.post(
            "/generator/command",
            &Command {
                command,
                parameters: vec![],
            },
        )
        .await
    }

    // ---- RTA ----

    pub async fn rta_configuration(&self) -> Result<RtaConfiguration, RewError> {
        self.get("/rta/configuration").await
    }

    pub async fn set_rta_configuration(
        &self,
        cfg: &RtaConfiguration,
    ) -> Result<ApiResponse, RewError> {
        self.post("/rta/configuration", cfg).await
    }

    /// "Start", "Stop", "Reset averaging", ... (`/rta/commands`).
    pub async fn rta_command(&self, command: &str) -> Result<ApiResponse, RewError> {
        self.post(
            "/rta/command",
            &Command {
                command,
                parameters: vec![],
            },
        )
        .await
    }

    /// Current RTA spectrum in `unit` (one of `/rta/captured-data/units`, e.g. "dBFS").
    /// REW encodes `magnitude` as base64 of big-endian f32, on a linear axis.
    pub async fn rta_captured_data(&self, unit: &str) -> Result<Spectrum, RewError> {
        let w: FrequencyResponseWire = self.get(&format!("/rta/captured-data?unit={unit}")).await?;
        if w.magnitude.is_empty() {
            return Err(RewError::NoData(
                w.message.unwrap_or_else(|| "empty magnitude".into()),
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(w.magnitude.as_bytes())
            .map_err(|e| RewError::Decode(e.to_string()))?;
        if bytes.len() % 4 != 0 {
            return Err(RewError::Decode(format!(
                "{} bytes is not a multiple of 4",
                bytes.len()
            )));
        }
        let magnitude = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_be_bytes(*b))
            .collect();
        Ok(Spectrum {
            unit: w.unit,
            start_hz: w.start_freq,
            step_hz: w.freq_step,
            magnitude,
            total_samples_processed: w.total_samples_processed,
        })
    }

    /// Live RTA distortion figures (needs `calcDistortionEnabled` and a sine).
    pub async fn rta_distortion(&self) -> Result<RtaDistortion, RewError> {
        let v: serde_json::Value = self.get("/rta/distortion").await?;
        // REW answers `[{"message": "No data"}]` (an array) while nothing is captured.
        let obj = match v {
            serde_json::Value::Array(mut a) if !a.is_empty() => a.remove(0),
            other => other,
        };
        let d: RtaDistortion =
            serde_json::from_value(obj).map_err(|e| RewError::Decode(e.to_string()))?;
        if d.thd.is_none() {
            return Err(RewError::NoData(
                d.message.unwrap_or_else(|| "no distortion data".into()),
            ));
        }
        Ok(d)
    }
}
