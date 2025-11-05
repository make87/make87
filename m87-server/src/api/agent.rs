use axum::extract::{Path, State};
use axum::routing::post;
use axum::{routing::get, Json, Router};
use mongodb::bson::doc;
use mongodb::bson::oid::ObjectId;
use tokio::join;

use crate::auth::claims::Claims;
use crate::auth::tunnel_token::issue_tunnel_token;
use crate::models::agent::{
    AgentDoc, HeartbeatRequest, HeartbeatResponse, PublicAgent, UpdateAgentBody,
};
use crate::models::roles::Role;
use crate::response::{ResponsePagination, ServerAppResult, ServerError, ServerResponse};
use crate::util::app_state::AppState;
use crate::util::pagination::RequestPagination;

pub fn create_route() -> Router<AppState> {
    Router::new()
        .route("/", get(get_agents))
        .route(
            "/{id}",
            get(get_agent_by_id)
                .post(update_agent_by_id)
                .delete(delete_agent),
        )
        .route("/{id}/heartbeat", post(post_heartbeat))
        .route("/{id}/logs", get(get_logs_websocket))
        .route("/{id}/terminal", get(get_terminal_websocket))
        .route("/{id}/metrics", get(get_metrics_websocket))
        .route("/{id}/ssh", get(get_agent_ssh))
        // .route("/{id}/forward", get(get_port_forward))
        .route("/{id}/token", get(get_tunnel_token))
}

async fn get_tunnel_token(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<String> {
    // only return to the agent itself
    if !claims.has_scope_and_role(&format!("agent:{}", id), Role::Editor) {
        return Err(ServerError::unauthorized("missing token"));
    }
    // 30s ttl should be enough to open a tunnel
    let token = issue_tunnel_token(&id, 30, &state.config.forward_secret)?;
    Ok(ServerResponse::builder().ok().body(token).build())
}

async fn get_agents(
    claims: Claims,
    State(state): State<AppState>,
    pagination: RequestPagination,
) -> ServerAppResult<Vec<PublicAgent>> {
    let agent_col = state.db.agents();
    let agents = claims.list_with_access(&agent_col, &pagination).await?;
    let total_count = claims.count_with_access(&agent_col).await?;

    let agents = PublicAgent::from_agents(&agents);

    Ok(ServerResponse::builder()
        .body(agents)
        .status_code(axum::http::StatusCode::OK)
        .pagination(ResponsePagination {
            count: total_count,
            offset: pagination.offset,
            limit: pagination.limit,
        })
        .build())
}

async fn get_agent_by_id(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<PublicAgent> {
    let agent_id =
        ObjectId::parse_str(&id).map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    let agent_opt = claims
        .find_one_with_access(&state.db.agents(), doc! { "_id": agent_id })
        .await?;
    let agent = agent_opt.ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let agent = PublicAgent::from_agent(&agent);

    Ok(ServerResponse::builder()
        .body(agent)
        .status_code(axum::http::StatusCode::OK)
        .build())
}

async fn update_agent_by_id(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAgentBody>,
) -> ServerAppResult<PublicAgent> {
    let agent_id =
        ObjectId::parse_str(&id).map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // Build the Mongo update document
    let update_doc = payload.to_update_doc(); // implement this helper on UpdateAgentBody

    // Execute authorized update
    claims
        .update_one_with_access(&state.db.agents(), doc! { "_id": agent_id }, update_doc)
        .await?;

    // Fetch the updated agent (using the same access filter)
    let updated_agent_opt = claims
        .find_one_with_access(&state.db.agents(), doc! { "_id": agent_id })
        .await?;

    let updated_agent = match updated_agent_opt {
        Some(agent) => agent,
        None => return Err(ServerError::not_found("Agent not found after update")),
    };

    let agent = PublicAgent::from_agent(&updated_agent);

    Ok(ServerResponse::builder()
        .body(agent)
        .status_code(axum::http::StatusCode::OK)
        .build())
}

async fn delete_agent(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<()> {
    let agent_oid = ObjectId::parse_str(&id)?;
    let agent_opt = claims
        .find_one_with_access(&state.db.agents(), doc! { "_id": agent_oid })
        .await?;
    let agent = agent_opt.ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let _ = agent.remove_agent(&claims, &state.db).await?;

    Ok(ServerResponse::builder()
        .status_code(axum::http::StatusCode::NO_CONTENT)
        .build())
}

async fn post_heartbeat(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<HeartbeatRequest>,
) -> ServerAppResult<HeartbeatResponse> {
    let agent = claims
        .find_one_with_scope_and_role::<AgentDoc>(
            &state.db.agents(),
            doc! { "_id": ObjectId::parse_str(&id)? },
            Role::Editor,
        )
        .await?
        .ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let body = agent.handle_heartbeat(claims, &state.db, payload).await?;
    let res = ServerResponse::builder().body(body).ok().build();
    Ok(res)
}

async fn get_agent_ssh(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<String> {
    let agent = claims
        .find_one_with_scope_and_role::<AgentDoc>(
            &state.db.agents(),
            doc! { "_id": ObjectId::parse_str(&id)? },
            Role::Editor,
        )
        .await?
        .ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let command = agent.request_ssh_command(&state).await?;
    let res = ServerResponse::builder().body(command).ok().build();
    Ok(res)
}

async fn get_logs_websocket(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<String> {
    let agent = claims
        .find_one_with_scope_and_role::<AgentDoc>(
            &state.db.agents(),
            doc! { "_id": ObjectId::parse_str(&id)? },
            Role::Editor,
        )
        .await?
        .ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let command = agent.get_logs_url(None, &state).await?;
    let res = ServerResponse::builder().body(command).ok().build();
    Ok(res)
}

async fn get_terminal_websocket(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<String> {
    let agent = claims
        .find_one_with_scope_and_role::<AgentDoc>(
            &state.db.agents(),
            doc! { "_id": ObjectId::parse_str(&id)? },
            Role::Editor,
        )
        .await?
        .ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let command = agent.get_terminal_url(None, &state).await?;
    let res = ServerResponse::builder().body(command).ok().build();
    Ok(res)
}

async fn get_metrics_websocket(
    claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ServerAppResult<String> {
    let agent = claims
        .find_one_with_scope_and_role::<AgentDoc>(
            &state.db.agents(),
            doc! { "_id": ObjectId::parse_str(&id)? },
            Role::Editor,
        )
        .await?
        .ok_or_else(|| ServerError::not_found("Agent not found"))?;

    let command = agent.get_metrics_url(None, &state).await?;
    let res = ServerResponse::builder().body(command).ok().build();
    Ok(res)
}
