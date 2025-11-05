use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    auth::{access_control::AccessControlled, claims::Claims},
    db::Mongo,
    response::{ServerError, ServerResult},
    util::{app_state::AppState, pagination::RequestPagination},
};

fn default_stable_version() -> String {
    "latest".to_string()
}

fn default_architecture() -> String {
    "unknown".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, Hash)]
pub struct AgentConfig {
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u32>,
    #[serde(default)]
    pub update_check_interval_secs: Option<u32>,
    pub server_port: u32,
}

impl From<AgentConfig> for Bson {
    fn from(state: AgentConfig) -> Self {
        mongodb::bson::to_bson(&state).unwrap()
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            heartbeat_interval_secs: Some(30),
            update_check_interval_secs: Some(60),
            server_port: 8337,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AgentSystemInfo {
    pub hostname: String,
    pub username: String,
    pub public_ip_address: Option<String>,
    pub operating_system: String,
    #[serde(default = "default_architecture")]
    pub architecture: String,
    #[serde(default)]
    pub cores: Option<u32>,
    pub cpu_name: String,
    #[serde(default)]
    /// Memory in GB
    pub memory: Option<f64>,
    #[serde(default)]
    pub gpus: Vec<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub country_code: Option<String>,
}

impl From<AgentSystemInfo> for Bson {
    fn from(state: AgentSystemInfo) -> Self {
        mongodb::bson::to_bson(&state).unwrap()
    }
}

impl Hash for AgentSystemInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hostname.hash(state);
        self.username.hash(state);
        self.public_ip_address.hash(state);
        self.operating_system.hash(state);
        self.architecture.hash(state);
        if let Some(cores) = &self.cores {
            cores.hash(state);
        }
        if let Some(memory) = &self.memory {
            memory.to_bits().hash(state);
        }
        if let Some(latitude) = &self.latitude {
            latitude.to_bits().hash(state);
        }
        if let Some(longitude) = &self.longitude {
            longitude.to_bits().hash(state);
        }
        self.country_code.hash(state);
        self.cpu_name.hash(state);
        self.gpus.hash(state);
    }
}

#[derive(Deserialize, Serialize, Hash, Default)]
pub struct UpdateAgentBody {
    pub system_info: Option<AgentSystemInfo>,
    pub agent_version: Option<String>,
    pub target_agent_version: Option<String>,
    #[serde(default)]
    pub agent_config: Option<AgentConfig>,
    #[serde(default)]
    pub owner_scope: Option<String>,
    #[serde(default)]
    pub allowed_scopes: Option<Vec<String>>,
}

impl UpdateAgentBody {
    pub fn to_update_doc(&self) -> Document {
        let mut update_fields = doc! {};

        if let Some(system_info) = &self.system_info {
            update_fields.insert("system_info", mongodb::bson::to_bson(system_info).unwrap());
        }

        if let Some(owner_scope) = &self.owner_scope {
            update_fields.insert("owner_scope", owner_scope);
        }

        if let Some(allowed_scopes) = &self.allowed_scopes {
            update_fields.insert("allowed_scopes", allowed_scopes);
        }

        if let Some(agent_version) = &self.agent_version {
            update_fields.insert("agent_version", agent_version);
        }

        if let Some(target_agent_version) = &self.target_agent_version {
            update_fields.insert("target_agent_version", target_agent_version);
        }

        if let Some(agent_config) = &self.agent_config {
            update_fields.insert(
                "agent_config",
                mongodb::bson::to_bson(agent_config).unwrap(),
            );
            // Force a compose recheck when config changes
            update_fields.insert("current_compose_hash", mongodb::bson::Bson::Null);
        }

        // Always set these system timestamps
        update_fields.insert("last_connection", DateTime::now());
        update_fields.insert("updated_at", DateTime::now());

        doc! { "$set": update_fields }
    }
}

#[derive(Deserialize, Serialize, Default)]
pub struct CreateAgentBody {
    pub id: Option<String>,
    pub name: String,
    pub target_client_version: Option<String>,
    pub owner_scope: String,
    pub allowed_scopes: Vec<String>,
    pub api_key_id: ObjectId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub short_id: String,
    pub name: String,
    pub updated_at: DateTime,
    pub created_at: DateTime,
    pub last_connection: DateTime,
    #[serde(default = "String::new")]
    pub agent_version: String,
    #[serde(default = "default_stable_version")]
    pub target_agent_version: String,
    #[serde(default)]
    pub agent_config: AgentConfig,
    pub owner_scope: String,
    pub allowed_scopes: Vec<String>,
    pub system_info: AgentSystemInfo,
    pub instruction_hash: i64,
    pub api_key_id: ObjectId,
}

impl AgentDoc {
    pub async fn create_from(db: &Arc<Mongo>, create_body: CreateAgentBody) -> ServerResult<()> {
        let agent_id = match create_body.id {
            Some(id) => ObjectId::parse_str(&id)?,
            None => ObjectId::new(),
        };
        // if format!("agent:{}", agent_id.to_string()) not in create_body.allowed_scopes add it
        let self_scope = format!("agent:{}", agent_id.to_string());
        // if !create_body.allowed_scopes.contains(&self_scope) {
        // create_body.allowed_scopes.push(self_scope);
        // }
        let allowed_scopes = match create_body.allowed_scopes.contains(&self_scope) {
            true => create_body.allowed_scopes,
            false => {
                let mut allowed_scopes = create_body.allowed_scopes;
                allowed_scopes.push(self_scope);
                allowed_scopes
            }
        };

        let now = DateTime::now();
        let agent = AgentDoc {
            id: Some(agent_id.clone()),
            short_id: short_agent_id(agent_id.to_string()),
            name: create_body.name,
            updated_at: now,
            created_at: now,
            last_connection: now,
            agent_version: "".to_string(),
            target_agent_version: "latest".to_string(),
            agent_config: AgentConfig::default(),
            owner_scope: create_body.owner_scope,
            allowed_scopes,
            system_info: AgentSystemInfo::default(),
            instruction_hash: 0,
            api_key_id: create_body.api_key_id,
        };
        let _ = db.agents().insert_one(agent).await?;

        Ok(())
    }

    pub async fn remove_agent(&self, claims: &Claims, db: &Arc<Mongo>) -> ServerResult<()> {
        let agents_col = db.agents();
        let api_keys_col = db.api_keys();
        let roles_col = db.roles();

        // Check access and delete agent
        claims
            .delete_one_with_access(&agents_col, doc! { "_id": self.id.clone().unwrap() })
            .await?;

        // Delete associated API keys
        api_keys_col
            .delete_many(doc! { "_id": self.api_key_id })
            .await
            .map_err(|_| ServerError::internal_error("Failed to delete API keys"))?;

        // Delete any roles scoped to this agent
        roles_col
            .delete_many(doc! { "reference_id": self.api_key_id })
            .await
            .map_err(|_| ServerError::internal_error("Failed to delete roles"))?;

        let success = claims
            .delete_one_with_access(&db.agents(), doc! { "_id": &self.id.clone().unwrap() })
            .await?;

        if success {
            return Err(ServerError::not_found(
                "Agent you are trying to remove does not exist",
            ));
        }
        Ok(())
    }

    pub async fn request_public_url(
        &self,
        name: &str,
        port: u16,
        url_prefix: &str,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let agent_id = self.id.clone().unwrap().to_string();
        let sni_host = match name.len() {
            0 => format!("{}.{}", self.short_id, state.config.public_address),
            _ => format!("{}.{}.{}", name, self.short_id, state.config.public_address),
        };
        let _ = state
            .relay
            .register_forward(sni_host.clone(), agent_id, port, allowed_source_ips);
        let url = format!("{}{}", url_prefix, sni_host,);
        Ok(url)
    }

    pub async fn request_ssh_command(&self, state: &AppState) -> ServerResult<String> {
        let url = self.request_public_url("ssh", 22, "", None, state).await?;
        let url = format!("ssh -p 443 make87@{}", url);
        Ok(url)
    }

    async fn get_agent_client_rest_url(
        &self,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let port = self.agent_config.server_port as u16;
        let url = self
            .request_public_url("", port, "https://", allowed_source_ips, state)
            .await?;
        Ok(url)
    }

    pub async fn get_logs_url(
        &self,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let url = self
            .get_agent_client_rest_url(allowed_source_ips, state)
            .await?;
        let url = format!("{}/logs", url);
        Ok(url)
    }

    pub async fn get_terminal_url(
        &self,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let url = self
            .get_agent_client_rest_url(allowed_source_ips, state)
            .await?;
        let url = format!("{}/terminal", url);
        Ok(url)
    }

    pub async fn get_container_terminal_url(
        &self,
        container_name: &str,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let url = self
            .get_agent_client_rest_url(allowed_source_ips, state)
            .await?;
        let url = format!("{}/container/{}", url, container_name);
        Ok(url)
    }

    pub async fn get_container_logs_url(
        &self,
        container_name: &str,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let url = self
            .get_agent_client_rest_url(allowed_source_ips, state)
            .await?;
        let url = format!("{}/container-logs/{}", url, container_name);
        Ok(url)
    }

    pub async fn get_metrics_url(
        &self,
        allowed_source_ips: Option<Vec<String>>,
        state: &AppState,
    ) -> ServerResult<String> {
        let url = self
            .get_agent_client_rest_url(allowed_source_ips, state)
            .await?;
        let url = format!("{}/metrics", url);
        Ok(url)
    }

    pub async fn handle_heartbeat(
        &self,
        claims: Claims,
        db: &Arc<Mongo>,
        payload: HeartbeatRequest,
    ) -> ServerResult<HeartbeatResponse> {
        let last_hash = payload.last_instruction_hash as i64;
        if self.instruction_hash == last_hash {
            return Ok(HeartbeatResponse::default());
        }

        let ssh_keys = claims
            .list_with_access(&db.ssh_keys(), &RequestPagination::max_limit())
            .await?;

        let ssh_keys = ssh_keys.into_iter().map(|key| key.key).collect();

        let config = self.agent_config.clone();
        let resp = HeartbeatResponse {
            compose_ref: None,
            client_config: Some(config),
            ssh_keys: Some(ssh_keys),
        };

        let new_hash = resp.get_hash() as i64;
        db.agents()
            .update_one(
                doc! {"_id": self.id.unwrap()},
                doc! {"$set": {"instruction_hash": new_hash}},
            )
            .await?;

        Ok(resp)
    }
}

fn short_agent_id(agent_id: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(agent_id.as_bytes());
    let hash = hex::encode(&hasher.finalize());
    let short = &hash[..6]; // 24 bits — should be enough entropy
    short.to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PublicAgent {
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub created_at: String,
    pub last_connection: String,
    pub online: bool,
    pub agent_version: String,
    pub target_agent_version: String,
    #[serde(default)]
    pub agent_config: AgentConfig,
    pub system_info: AgentSystemInfo,
}

impl PublicAgent {
    pub fn from_agent(agent: &AgentDoc) -> Self {
        let now_ms = DateTime::now().timestamp_millis();
        let last_ms = agent.last_connection.timestamp_millis();
        let heartbeat_secs = agent
            .agent_config
            .heartbeat_interval_secs
            .clone()
            .unwrap_or(30);
        // convert u32 to i64
        let heartbeat_secs = heartbeat_secs as i64;

        let online = now_ms - last_ms < 3 * heartbeat_secs * 1000;
        Self {
            id: agent.id.unwrap().to_string(),
            name: agent.name.clone(),
            updated_at: agent.updated_at.try_to_rfc3339_string().unwrap(),
            created_at: agent.created_at.try_to_rfc3339_string().unwrap(),
            last_connection: agent.last_connection.try_to_rfc3339_string().unwrap(),
            online,
            agent_version: agent.agent_version.clone(),
            target_agent_version: agent.target_agent_version.clone(),
            agent_config: agent.agent_config.clone(),
            system_info: agent.system_info.clone(),
        }
    }

    pub fn from_agents(agents: &Vec<AgentDoc>) -> Vec<Self> {
        agents.iter().map(Self::from_agent).collect()
    }
}

impl AccessControlled for AgentDoc {
    fn owner_scope_field() -> &'static str {
        "owner_scope"
    }
    fn allowed_scopes_field() -> Option<&'static str> {
        Some("allowed_scopes")
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HeartbeatRequest {
    pub last_instruction_hash: u64,
    pub needs_nexus_token: bool,
    // pub system: SystemMetrics,
    // pub services: Vec<ServiceInfo>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct HeartbeatResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_ref: Option<String>,
    // pub digests: Option<Digests>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_config: Option<AgentConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_keys: Option<Vec<String>>,
}

impl Hash for HeartbeatResponse {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.compose_ref.hash(state);
        self.client_config.hash(state);
        self.ssh_keys.hash(state);
    }
}

impl HeartbeatResponse {
    pub fn get_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}
