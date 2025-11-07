use crate::{
    db::Mongo,
    response::{ServerError, ServerResult},
    util::pagination::RequestPagination,
};
use futures::StreamExt;
use m87_shared::forward::{CreateForward, ForwardAccess, ForwardUpdateRequest, PublicForward};
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    options::{FindOneAndUpdateOptions, FindOptions, IndexOptions, ReturnDocument},
    IndexModel,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn default_access() -> ForwardAccess {
    ForwardAccess::Open
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,

    /// Reference to the device this forward belongs to
    pub device_id: ObjectId,

    /// Short identifier of the device (for subdomain parsing)
    pub device_short_id: String,

    /// Optional human-readable name for the forward
    pub name: Option<String>,

    /// Target port on the device
    pub target_port: u32,

    #[serde(default = "default_access")]
    pub access: ForwardAccess,

    #[serde(default)]
    pub created_at: Option<DateTime>,

    #[serde(default)]
    pub updated_at: Option<DateTime>,
}

impl ForwardDoc {
    /// Create or update a forward by (device_id, target_port)
    pub async fn create_or_update(db: &Arc<Mongo>, body: CreateForward) -> ServerResult<Self> {
        let now = DateTime::now();
        let device_oid = ObjectId::parse_str(body.device_id)?;
        let filter = doc! {
            "device_id": &device_oid,
            "target_port": body.target_port as u32,
        };

        let update = doc! {
            "$set": {
                "device_short_id": &body.device_short_id,
                "name": &body.name,
                "access": mongodb::bson::to_bson(&body.access).unwrap(),
                "updated_at": now,
            },
            "$setOnInsert": {
                "device_id": &device_oid,
                "created_at": now,
            }
        };

        let options = FindOneAndUpdateOptions::builder()
            .upsert(true)
            .return_document(ReturnDocument::After)
            .build();

        let coll = db.forwards();
        let updated = coll
            .find_one_and_update(filter, update)
            .with_options(options)
            .await
            .map_err(|e| ServerError::internal_error(&format!("Failed to upsert forward: {e}")))?;

        Ok(updated.unwrap())
    }

    pub async fn list_all(db: &Arc<Mongo>) -> ServerResult<Vec<ForwardDoc>> {
        let mut cursor = db
            .forwards()
            .find(doc! {})
            .await
            .map_err(|_| ServerError::internal_error("Failed to list forwards"))?;

        let mut items = Vec::new();
        while let Some(res) = cursor.next().await {
            match res {
                Ok(doc) => items.push(doc),
                Err(_) => return Err(ServerError::internal_error("Failed to decode forward doc")),
            }
        }
        Ok(items)
    }

    pub async fn list_for_device(
        db: &Arc<Mongo>,
        device_id: &ObjectId,
        pagination: &RequestPagination,
    ) -> ServerResult<Vec<ForwardDoc>> {
        let mut cursor = db
            .forwards()
            .find(doc! { "device_id": device_id })
            .with_options(
                FindOptions::builder()
                    .skip(pagination.offset)
                    .limit(Some(pagination.limit as i64))
                    .build(),
            )
            .await
            .map_err(|_| ServerError::internal_error("Failed to list device forwards"))?;

        let mut items = Vec::new();
        while let Some(res) = cursor.next().await {
            match res {
                Ok(doc) => items.push(doc),
                Err(_) => return Err(ServerError::internal_error("Failed to decode forward doc")),
            }
        }
        Ok(items)
    }

    pub async fn get_by_port(
        db: &Arc<Mongo>,
        device_id: &ObjectId,
        target_port: u16,
    ) -> ServerResult<Option<ForwardDoc>> {
        let item = db
            .forwards()
            .find_one(doc! { "device_id": device_id, "target_port": target_port as u32 })
            .await?;
        Ok(item)
    }

    pub async fn delete(
        db: &Arc<Mongo>,
        device_id: &ObjectId,
        target_port: u16,
    ) -> ServerResult<()> {
        db.forwards()
            .delete_one(doc! { "device_id": device_id, "target_port": target_port as u32 })
            .await
            .map_err(|_| ServerError::internal_error("Failed to delete forward"))?;
        Ok(())
    }

    pub async fn update(
        db: &Arc<Mongo>,
        device_id: &ObjectId,
        target_port: u16,
        update: ForwardUpdateRequest,
    ) -> ServerResult<()> {
        let filter = doc! { "device_id": device_id, "target_port": target_port as u32 };

        let mut udpate_fields = doc! {};
        if let Some(name) = update.name {
            udpate_fields.insert("name", name);
        }
        if let Some(target_port) = update.target_port {
            udpate_fields.insert("target_port", target_port as u32);
        }
        if let Some(access) = update.access {
            udpate_fields.insert("access", mongodb::bson::to_bson(&access).unwrap());
        }

        let update = doc! { "$set": udpate_fields };

        let updated = db
            .forwards()
            .update_one(filter, update)
            .await
            .map_err(|_| ServerError::internal_error("Failed to update forward"))?;

        if updated.modified_count == 0 {
            Err(ServerError::not_found("Forward not found"))
        } else {
            Ok(())
        }
    }
}

/// Add this to Mongo::ensure_indexes()
pub async fn ensure_forward_indexes(db: &Arc<Mongo>) -> ServerResult<()> {
    db.forwards()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "device_id": 1, "target_port": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

    db.forwards()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "device_short_id": 1 })
                .build(),
        )
        .await?;

    Ok(())
}

impl Into<PublicForward> for ForwardDoc {
    fn into(self) -> PublicForward {
        PublicForward {
            id: self.id.unwrap().to_string(),
            device_id: self.device_id.to_string(),
            device_short_id: self.device_short_id,
            name: self.name,
            target_port: self.target_port as u16,
            access: self.access,
            created_at: self.created_at.map(|t| t.try_to_rfc3339_string().unwrap()),
            updated_at: self.updated_at.map(|t| t.try_to_rfc3339_string().unwrap()),
        }
    }
}
