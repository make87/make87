use std::sync::Arc;

use futures::TryStreamExt;
use m87_shared::deploy_spec::{
    DeployReport, DeployReportKind, DeploymentRevision, DeploymentRevisionReport, RollbackReport,
    RunReport, RunSpec, RunState, StepReport,
};
use mongodb::{
    bson::{Bson, DateTime as BsonDateTime, Document, doc, oid::ObjectId, to_bson},
    options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument},
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::access_control::AccessControlled,
    db::Mongo,
    response::{ServerError, ServerResult},
    util::pagination::RequestPagination,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployRevisionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub revision: DeploymentRevision,
    #[serde(default)]
    pub device_id: Option<ObjectId>,
    // placeholder for later
    #[serde(default)]
    pub group_id: Option<ObjectId>,

    pub active: bool,
    pub dirty: bool,
    pub index: u32,

    pub owner_scope: String,
    pub allowed_scopes: Vec<String>,
}

impl AccessControlled for DeployRevisionDoc {
    fn owner_scope_field() -> &'static str {
        "owner_scope"
    }
    fn allowed_scopes_field() -> Option<&'static str> {
        Some("allowed_scopes")
    }
}

#[derive(Deserialize, Serialize, Default)]
pub struct UpdateDeployRevisionBody {
    #[serde(default)]
    pub revision: Option<String>,
    // yaml of the new run spec
    #[serde(default)]
    pub add_run_spec: Option<String>,
    // yaml of the updated run spec
    #[serde(default)]
    pub update_run_spec: Option<String>,
    // id of the run spec to remove
    #[serde(default)]
    pub remove_run_spec_id: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
}

impl UpdateDeployRevisionBody {
    pub fn to_update_doc(&self) -> ServerResult<(Document, Option<Document>)> {
        let mut which = 0;
        if self.revision.is_some() {
            which += 1;
        }
        if self.add_run_spec.is_some() {
            which += 1;
        }
        if self.update_run_spec.is_some() {
            which += 1;
        }
        if self.remove_run_spec_id.is_some() {
            which += 1;
        }
        if self.active.is_some() {
            which += 1;
        }

        if which == 0 {
            return Err(ServerError::bad_request("Missing fields"));
        }
        if which > 1 {
            return Err(ServerError::bad_request(
                "only one field may be set per update",
            ));
        }

        // 1) replace whole revision
        if let Some(yaml) = &self.revision {
            let rev: DeploymentRevision = serde_yaml::from_str(yaml).map_err(|e| {
                ServerError::bad_request(&format!("invalid YAML in `revision`: {}", e))
            })?;
            return Ok((
                doc! { "$set": { "revision": to_bson(&rev).map_err(|e| ServerError::bad_request(&format!("revision -> bson failed: {}", e)))? } },
                None,
            ));
        }

        // 2) add one RunSpec
        if let Some(yaml) = &self.add_run_spec {
            let rs: RunSpec = serde_yaml::from_str(yaml).map_err(|e| {
                ServerError::bad_request(&format!("invalid YAML in `add_run_spec`: {}", e))
            })?;
            return Ok((
                doc! { "$push": { "revision.units": to_bson(&rs).map_err(|e| ServerError::bad_request(&format!("RunSpec -> bson failed: {}", e)))? } },
                None,
            ));
        }

        // 3) update/replace one RunSpec by id (positional `$`)
        if let Some(yaml) = &self.update_run_spec {
            let rs: RunSpec = serde_yaml::from_str(yaml).map_err(|e| {
                ServerError::bad_request(&format!("invalid YAML in `update_run_spec`: {}", e))
            })?;
            let id = rs.id.as_deref().ok_or_else(|| {
                ServerError::bad_request("`update_run_spec` RunSpec missing `id`")
            })?;
            return Ok((
                doc! { "$set": { "revision.units.$": to_bson(&rs).map_err(|e| ServerError::bad_request(&format!("RunSpec -> bson failed: {}", e)))? } },
                Some(doc! { "revision.units.id": id }),
            ));
        }

        // 4) remove one RunSpec by id
        if let Some(id) = &self.remove_run_spec_id {
            return Ok((doc! { "$pull": { "revision.units": { "id": id } } }, None));
        }

        // 5) set active
        if let Some(active) = self.active {
            return Ok((doc! { "$set": { "active": active } }, None));
        }

        Err(ServerError::internal_error("This should be unreachable"))
    }
}

impl DeployRevisionDoc {
    pub async fn create(
        db: &Arc<Mongo>,
        revision: DeploymentRevision,
        device_id: Option<ObjectId>,
        group_id: Option<ObjectId>,
        active: bool,
        owner_scope: String,
        allowed_scopes: Vec<String>,
    ) -> ServerResult<Self> {
        // index is the cnt of current docs for the dive or group
        let index = match (device_id, group_id) {
            (Some(device_id), _) => db
                .deploy_revisions()
                .count_documents(doc! {"device_id": device_id})
                .await
                .unwrap_or(0) as u32,
            (None, Some(group_id)) => db
                .deploy_revisions()
                .count_documents(doc! {"group_id": group_id})
                .await
                .unwrap_or(0) as u32,
            _ => {
                return Err(ServerError::bad_request(
                    "Either device_id or group_id must be provided",
                ));
            }
        };

        let doc = Self {
            id: None,
            revision,
            device_id,
            group_id,
            active,
            dirty: false,
            index,
            owner_scope,
            allowed_scopes,
        };
        db.deploy_revisions()
            .insert_one(&doc)
            .await
            .map_err(|_| ServerError::internal_error("Failed to insert API key"))?;
        Ok(doc)
    }

    pub async fn get_active_device_deployment(
        db: &Arc<Mongo>,
        device_id: ObjectId,
    ) -> ServerResult<Self> {
        let doc_opt = db
            .deploy_revisions()
            .find_one(doc! { "device_id": device_id, "active": true })
            .await?;

        match doc_opt {
            Some(d) => Ok(d),
            None => Err(ServerError::bad_request("Failed to find active deployment")),
        }
    }

    pub async fn list_for_device(
        db: &Arc<Mongo>,
        device_id: ObjectId,
        pagination: &RequestPagination,
    ) -> ServerResult<Vec<DeployRevisionDoc>> {
        let options = FindOptions::builder()
            .skip(Some(pagination.offset))
            .limit(Some(pagination.limit as i64))
            // sort by index descending
            .sort(doc! {"index": -1})
            .build();
        let cursor = db
            .deploy_revisions()
            .find(doc! { "device_id": device_id })
            .with_options(options)
            .await?;
        let results: Vec<DeployRevisionDoc> = cursor
            .try_collect()
            .await
            .map_err(|_| ServerError::internal_error("Cursor decode failed"))?;
        Ok(results)
    }

    pub async fn get_by_revision_id(
        db: &Arc<Mongo>,
        revision_id: String,
    ) -> ServerResult<DeployRevisionDoc> {
        let doc = db
            .deploy_revisions()
            .find_one(doc! { "revision.id": revision_id })
            .await?;
        doc.ok_or(ServerError::not_found("Deploy revision not found"))
    }
}

fn parse_yaml_revision(yaml: &str) -> ServerResult<DeploymentRevision> {
    serde_yaml::from_str(yaml)
        .map_err(|e| ServerError::bad_request(&format!("Failed to parse YAML: {}", e)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployReportDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,

    pub device_id: ObjectId,
    pub revision_id: String,

    pub kind: DeployReportKind,

    /// TTL target (Mongo will delete when this time is reached)
    pub expires_at: Option<BsonDateTime>,

    /// When the report was received/created
    pub created_at: BsonDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDeployReportBody {
    pub device_id: ObjectId,
    pub revision_id: String,
    pub kind: DeployReportKind,

    /// Optional TTL (server can set a default if None)
    #[serde(default)]
    pub expires_at: Option<BsonDateTime>,
}

impl DeployReportDoc {
    fn upsert_filter(body: &CreateDeployReportBody) -> ServerResult<Document> {
        let base = doc! {
            "device_id": body.device_id,
            "revision_id": &body.revision_id,
        };

        let f = match &body.kind {
            DeployReportKind::DeploymentRevisionReport(_) => {
                doc! { "kind.type": "DeploymentRevisionReport" }
            }
            DeployReportKind::RunReport(r) => {
                doc! {
                    "kind.type": "RunReport",
                    "kind.data.run_id": &r.run_id,
                }
            }
            DeployReportKind::StepReport(s) => {
                // Match on (run_id, name, attempts) as you requested.
                // name is Option<String> -> match null when None.
                let name_bson = match &s.name {
                    Some(n) => Bson::String(n.clone()),
                    None => Bson::Null,
                };

                doc! {
                    "kind.type": "StepReport",
                    "kind.data.run_id": &s.run_id,
                    "kind.data.name": name_bson,
                    "kind.data.attempts": Bson::Int32(s.attempts as i32),
                }
            }
            DeployReportKind::RollbackReport(_) => {
                // If you can have multiple rollback reports per revision and want only one, this is fine.
                // If you want to key by something else, add it here.
                doc! { "kind.type": "RollbackReport" }
            }
            DeployReportKind::RunState(s) => {
                doc! {
                    "kind.type": "RunState",
                    "kind.data.run_id": &s.run_id,
                }
            }
        };

        Ok(doc! { "$and": [base, f] })
    }

    pub async fn create_or_update(
        db: &Arc<Mongo>,
        body: CreateDeployReportBody,
    ) -> ServerResult<Self> {
        let filter = Self::upsert_filter(&body)?;

        let now = BsonDateTime::now();
        let kind_bson = to_bson(&body.kind)
            .map_err(|_| ServerError::internal_error("Failed to serialize deploy report kind"))?;

        // Overwrite the report doc (except _id) on every update.
        // created_at becomes "received_at" semantics (latest receive time).
        let update = doc! {
            "$set": {
                "device_id": &body.device_id,
                "revision_id": &body.revision_id,
                "kind": kind_bson,
                "expires_at": body.expires_at,
                "created_at": now,
            },
            // Ensures the fields exist on insert even if you later change $set semantics.
            "$setOnInsert": {
                "device_id": &body.device_id,
                "revision_id": &body.revision_id,
            }
        };

        let opts = FindOneAndUpdateOptions::builder()
            .upsert(true)
            .return_document(ReturnDocument::After)
            .build();

        let doc = db
            .deploy_reports()
            .find_one_and_update(filter, update)
            .with_options(opts)
            .await
            .map_err(|_| ServerError::internal_error("Failed to upsert deploy report"))?
            .ok_or_else(|| ServerError::internal_error("Upsert returned no document"))?;

        Ok(doc)
    }

    pub fn to_pub_report(&self) -> DeployReport {
        DeployReport {
            device_id: self.device_id.to_string(),
            revision_id: self.revision_id.clone(),
            kind: self.kind.clone(),
            expires_at: self.expires_at.map(|dt| dt.timestamp_millis() as u64),
            created_at: self.created_at.timestamp_millis() as u64,
        }
    }

    pub async fn delete(db: &Arc<Mongo>, id: ObjectId) -> ServerResult<bool> {
        let res = db
            .deploy_reports()
            .delete_one(doc! { "_id": id })
            .await
            .map_err(|_| ServerError::internal_error("Failed to delete deploy report"))?;
        Ok(res.deleted_count == 1)
    }

    pub async fn list_for_device(
        db: &Arc<Mongo>,
        device_id: &ObjectId,
        revision_id: &str,
        pagination: &RequestPagination,
    ) -> ServerResult<Vec<Self>> {
        let options = FindOptions::builder()
            .skip(Some(pagination.offset))
            .limit(Some(pagination.limit as i64))
            .build();
        let cursor = db
            .deploy_reports()
            .find(doc! { "device_id": device_id, "revision_id": revision_id })
            .with_options(options)
            .await?;
        let results: Vec<DeployReportDoc> = cursor
            .try_collect()
            .await
            .map_err(|_| ServerError::internal_error("Cursor decode failed"))?;
        Ok(results)
    }

    pub async fn mark_revision_seen(
        db: &Arc<Mongo>,
        device_id: ObjectId,
        revision_id: &str,
        seen: bool,
    ) -> ServerResult<u64> {
        let res = db
            .deploy_reports()
            .update_many(
                doc! { "device_id": device_id, "revision_id": revision_id },
                doc! { "$set": { "seen": seen } },
            )
            .await
            .map_err(|_| ServerError::internal_error("Failed to update deploy reports"))?;

        Ok(res.modified_count)
    }
}
