use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime};

use serde::{Deserialize, Serialize};

use crate::{
    auth::access_control::AccessControlled,
    db::Mongo,
    response::{ServerError, ServerResult},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuthRequestDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    /// uuid of the request
    pub request_id: String,
    /// fastfetch string output
    pub agent_info: String,
    /// Time when the entry was created
    pub created_at: DateTime,
    pub agent_id: String,
    pub hostname: String,
    pub owner_scope: String,
    pub approved: bool,
}

impl AgentAuthRequestDoc {
    pub async fn create(db: &Arc<Mongo>, body: AgentAuthRequestBody) -> ServerResult<String> {
        let request_uuid = uuid::Uuid::new_v4().to_string();
        let request = AgentAuthRequestDoc {
            id: None,
            request_id: request_uuid.clone(),
            created_at: DateTime::now(),
            agent_info: body.agent_info.to_string(),
            agent_id: body.agent_id.to_string(),
            owner_scope: body.owner_scope.to_string(),
            hostname: body.hostname.to_string(),
            approved: false,
        };
        let _ = db
            .agent_auth_requests()
            .insert_one(request)
            .await
            .map_err(|err| {
                tracing::error!("Failed to create agent auth request: {}", err);
                ServerError::internal_error("Failed to create agent auth request")
            })?;

        Ok(request_uuid)
    }
}

impl AccessControlled for AgentAuthRequestDoc {
    fn owner_scope_field() -> &'static str {
        "owner_scope"
    }

    fn allowed_scopes_field() -> Option<&'static str> {
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PublicAgentAuthRequest {
    pub request_id: String,
    pub agent_info: String,
    pub created_at: String,
}

impl From<AgentAuthRequestDoc> for PublicAgentAuthRequest {
    fn from(request: AgentAuthRequestDoc) -> Self {
        PublicAgentAuthRequest {
            request_id: request.request_id,
            agent_info: request.agent_info,
            created_at: request.created_at.try_to_rfc3339_string().unwrap(),
        }
    }
}

impl PublicAgentAuthRequest {
    pub fn from_vec(agents: Vec<AgentAuthRequestDoc>) -> Vec<Self> {
        agents.into_iter().map(Into::into).collect()
    }
}

#[derive(Serialize, Deserialize)]
pub struct AgentAuthRequestBody {
    pub agent_info: String,
    pub hostname: String,
    pub owner_scope: String,
    pub agent_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentAuthRequestCheckResponse {
    pub state: String,
    pub api_key: Option<String>,
}

#[derive(Deserialize)]
pub struct AuthRequestAction {
    pub accept: bool,
    pub request_id: String,
}

#[derive(Deserialize)]
pub struct CheckAuthRequest {
    pub request_id: String,
}
