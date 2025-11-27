use anyhow::Result;
use m87_shared::device::PublicDevice;

use crate::{auth::AuthManager, config::Config, server};

pub async fn list_devices() -> Result<Vec<PublicDevice>> {
    let token = AuthManager::get_cli_token().await?;
    let config = Config::load()?;
    server::list_devices(
        &config.get_server_url(),
        &token,
        config.trust_invalid_server_cert,
    )
    .await
}

pub async fn get_device_by_name(name: &str) -> Result<PublicDevice> {
    list_devices()
        .await?
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| anyhow::anyhow!("Device '{}' not found", name))
}
