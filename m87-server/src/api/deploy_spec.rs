use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use m87_shared::deploy_spec::DeployReport;
use mongodb::bson::{doc, oid::ObjectId};
use serde::Deserialize;

use crate::auth::claims::Claims;
use crate::models::deploy_spec::{DeployReportDoc, DeployRevisionDoc, UpdateDeployRevisionBody};
use crate::response::{ResponsePagination, ServerAppResult, ServerError, ServerResponse};
use crate::util::app_state::AppState;
use crate::util::pagination::RequestPagination;

pub fn create_route() -> Router<AppState> {
    // This router is mounted under /devices already.
    Router::new()
        // Revisions
        .route(
            "/{device_id}/revisions",
            get(list_device_revisions).post(create_device_revision),
        )
        .route(
            "/{device_id}/revisions/{id}",
            get(get_revision_by_id)
                .post(update_revision_by_id)
                .delete(delete_revision),
        )
        .route(
            "/{device_id}/revisions/active",
            get(get_device_active_revision_id),
        )
        .route(
            "/{device_id}/revisions/{revision_id}/reports",
            get(list_device_revision_reports),
        )
}

#[derive(Debug, Deserialize)]
struct CreateDeployRevisionBody {
    /// YAML string for DeploymentRevision.
    pub revision: String,
    #[serde(default)]
    pub active: Option<bool>,
}

async fn list_device_revisions(
    claims: Claims,
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    pagination: RequestPagination,
) -> ServerAppResult<Vec<String>> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // Ensure caller can access the device
    let dev_opt = claims
        .find_one_with_access(&state.db.deploy_revisions(), doc! { "_id": device_oid })
        .await?;
    if dev_opt.is_none() {
        return Err(ServerError::not_found("Device not found"));
    }

    let docs = DeployRevisionDoc::list_for_device(&state.db, device_oid, &pagination).await?;
    let total_count = docs.len() as u64;
    let out: Vec<String> = docs
        .into_iter()
        .map(|doc| serde_yaml::to_string(&doc.revision))
        .filter_map(|s| s.ok())
        .collect();

    Ok(ServerResponse::builder()
        .body(out)
        .status_code(axum::http::StatusCode::OK)
        .pagination(ResponsePagination {
            count: total_count,
            offset: pagination.offset,
            limit: pagination.limit,
        })
        .build())
}

async fn get_device_active_revision_id(
    claims: Claims,
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> ServerAppResult<Option<String>> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // Ensure caller can access the device
    let dev_opt = claims
        .find_one_with_access(&state.db.devices(), doc! { "_id": device_oid })
        .await?;
    if dev_opt.is_none() {
        return Err(ServerError::not_found("Device not found"));
    }

    let out = DeployRevisionDoc::get_active_device_deployment(&state.db, device_oid).await?;

    Ok(ServerResponse::builder()
        .body(out.revision.id)
        .status_code(axum::http::StatusCode::OK)
        .build())
}

async fn create_device_revision(
    claims: Claims,
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(payload): Json<CreateDeployRevisionBody>,
) -> ServerAppResult<String> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // Ensure caller can access the device
    let dev_opt = claims
        .find_one_with_access(&state.db.devices(), doc! { "_id": device_oid })
        .await?;
    if dev_opt.is_none() {
        return Err(ServerError::not_found("Device not found"));
    }
    let device = dev_opt.unwrap();

    let revision: m87_shared::deploy_spec::DeploymentRevision =
        m87_shared::deploy_spec::DeploymentRevision::from_yaml(&payload.revision)
            .map_err(|e| ServerError::internal_error(&format!("{:?}", e)))?;

    let doc = DeployRevisionDoc::create(
        &state.db,
        revision,
        Some(device_oid),
        None,
        payload.active.unwrap_or(true),
        device.owner_scope,
        device.allowed_scopes,
    )
    .await?;

    let yml = doc
        .revision
        .to_yaml()
        .map_err(|e| ServerError::internal_error(&format!("{:?}", e)))?;

    Ok(ServerResponse::builder()
        .body(yml)
        .status_code(axum::http::StatusCode::CREATED)
        .build())
}

async fn get_revision_by_id(
    claims: Claims,
    State(state): State<AppState>,
    Path((device_id, id)): Path<(String, String)>,
) -> ServerAppResult<String> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // Ensure caller can access device
    let doc_opt = claims
        .find_one_with_access(
            &state.db.deploy_revisions(),
            doc! { "revision.id": id, "device_id": device_oid},
        )
        .await?;
    if doc_opt.is_none() {
        return Err(ServerError::not_found("Deployment Revision not found"));
    }

    let doc = doc_opt.ok_or_else(|| ServerError::not_found("Revision not found"))?;
    let yml = doc
        .revision
        .to_yaml()
        .map_err(|_| ServerError::internal_error("Failed to serialize YAML"))?;
    Ok(ServerResponse::builder()
        .body(yml)
        .status_code(axum::http::StatusCode::OK)
        .build())
}

async fn update_revision_by_id(
    claims: Claims,
    State(state): State<AppState>,
    Path((device_id, id)): Path<(String, String)>,
    Json(payload): Json<UpdateDeployRevisionBody>,
) -> ServerAppResult<()> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    let (update_doc, extra_filter) = payload.to_update_doc()?;

    let mut filter = doc! { "revision.id": id, "device_id": device_oid };
    if let Some(extra) = extra_filter {
        filter.extend(extra);
    }
    let success = claims
        .update_one_with_access::<DeployRevisionDoc>(
            &state.db.deploy_revisions(),
            filter,
            update_doc,
        )
        .await?;

    if !success {
        return Err(ServerError::not_found("Revision not found"));
    }

    Ok(ServerResponse::builder()
        .status_code(axum::http::StatusCode::NO_CONTENT)
        .build())
}

async fn delete_revision(
    claims: Claims,
    State(state): State<AppState>,
    Path((device_id, id)): Path<(String, String)>,
) -> ServerAppResult<()> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // authorize by selecting first
    let success = claims
        .delete_one_with_access(
            &state.db.deploy_revisions(),
            doc! { "revision.id": id, "device_id": device_oid },
        )
        .await?;
    if !success {
        return Err(ServerError::not_found("Revision not found"));
    }

    Ok(ServerResponse::builder()
        .status_code(axum::http::StatusCode::NO_CONTENT)
        .build())
}

async fn list_device_revision_reports(
    claims: Claims,
    State(state): State<AppState>,
    Path((device_id, revision_id)): Path<(String, String)>,
    pagination: RequestPagination,
) -> ServerAppResult<Vec<DeployReport>> {
    let device_oid = ObjectId::parse_str(&device_id)
        .map_err(|_| ServerError::bad_request("Invalid ObjectId"))?;

    // Ensure caller can access the device
    let dev_opt = claims
        .find_one_with_access(&state.db.devices(), doc! { "_id": &device_oid })
        .await?;
    if dev_opt.is_none() {
        return Err(ServerError::not_found("Device not found"));
    }

    let docs =
        DeployReportDoc::list_for_device(&state.db, &device_oid, &revision_id, &pagination).await?;
    let reports: Vec<DeployReport> = docs.into_iter().map(|doc| doc.to_pub_report()).collect();
    let total_count = reports.len() as u64;

    Ok(ServerResponse::builder()
        .body(reports)
        .status_code(axum::http::StatusCode::OK)
        .pagination(ResponsePagination {
            count: total_count,
            offset: pagination.offset,
            limit: pagination.limit,
        })
        .build())
}
