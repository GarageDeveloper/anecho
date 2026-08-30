use thiserror::Error;

/// Errors surfaced by device backends.
#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("device not found: {0}")]
    NotFound(String),
    #[error("device is busy or already streaming")]
    Busy,
    #[error("unsupported configuration: {0}")]
    UnsupportedConfig(String),
    #[error("device not configured; call configure() first")]
    NotConfigured,
    #[error("no stream with this handle")]
    NoSuchStream,
    #[error("device disconnected")]
    Disconnected,
    #[error("backend error: {0}")]
    Backend(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
