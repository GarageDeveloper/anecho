//! Anecho side of the bench: a thin wrapper over `anecho-client`.

pub use anecho_client::Client;
use anecho_contract::v0 as pb;

pub const DEFAULT_URL: &str = "ws://127.0.0.1:4800/ws";

#[derive(Debug, Clone)]
pub struct Summary {
    pub backend_version: String,
    pub contract_version: String,
    pub devices: Vec<pb::DeviceInfo>,
}

pub async fn summary(url: &str) -> anecho_client::Result<Summary> {
    let client = Client::connect(url).await?;
    let v = client.version().await?;
    let devices = client.list_devices().await?;
    Ok(Summary {
        backend_version: v.backend_version,
        contract_version: v.contract_version,
        devices,
    })
}
