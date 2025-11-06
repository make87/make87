use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ForwardAccess {
    /// No restrictions
    Open,
    /// Only these IPs are allowed
    IpWhitelist(Vec<String>),
    /// Requires a valid bearer/JWT token
    Auth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicForward {
    pub id: String,
    pub device_id: String,
    pub device_short_id: String,
    pub name: Option<String>,
    pub target_port: u16,
    pub access: ForwardAccess,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateForward {
    pub device_id: String,
    pub device_short_id: String,
    pub name: Option<String>,
    pub target_port: u16,
    pub access: ForwardAccess,
}
