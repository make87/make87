use std::{sync::Arc, time::Duration};

use futures::TryStreamExt;
use m87_shared::device::AuditLog;
use mongodb::{
    bson::{DateTime, doc, oid::ObjectId},
    options::FindOptions,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::claims::Claims,
    config::AppConfig,
    db::Mongo,
    response::{ServerError, ServerResult},
    util::pagination::RequestPagination,
};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AuditLogDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub timestamp: DateTime,
    pub user_id: Option<ObjectId>,
    pub user_name: String,
    pub user_mail: String,
    pub action: String,
    pub details: String,
    pub device_id: Option<ObjectId>,
    #[serde(default)]
    pub expires_at: Option<DateTime>,
}

impl AuditLogDoc {
    pub async fn add(
        db: &Arc<Mongo>,
        claims: &Claims,
        config: &Arc<AppConfig>,
        action: &str,
        details: &str,
        device_id: Option<ObjectId>,
    ) -> ServerResult<()> {
        let expires_at = Some(DateTime::from_system_time(
            DateTime::now().to_system_time()
                + Duration::from_hours((config.audit_retention_days * 24) as u64),
        ));

        let doc: AuditLogDoc = Self {
            id: None,
            timestamp: DateTime::now(),
            user_id: claims.user_id.clone(),
            user_name: claims.user_name.clone(),
            user_mail: claims.user_email.clone(),
            action: action.to_string(),
            details: details.to_string(),
            device_id,
            expires_at,
        };
        db.audit_logs()
            .insert_one(&doc)
            .await
            .map_err(|_| ServerError::internal_error("Failed to insert API key"))?;
        Ok(())
    }

    pub async fn list_for_device(
        db: &Arc<Mongo>,
        device_id: ObjectId,
        pagination: &RequestPagination,
    ) -> ServerResult<Vec<AuditLogDoc>> {
        let mut filter = doc! { "device_id": device_id };
        if pagination.since.is_some() || pagination.until.is_some() {
            let mut ts = mongodb::bson::Document::new();
            if let Some(since) = pagination.since {
                ts.insert("$gte", since);
            }
            if let Some(until) = pagination.until {
                ts.insert("$lte", until);
            }
            filter.insert("timestamp", ts);
        }

        let options = FindOptions::builder()
            .skip(Some(pagination.offset))
            .limit(Some(pagination.limit as i64))
            .sort(doc! { "timestamp": -1 })
            .build();

        let cursor = db.audit_logs().find(filter).with_options(options).await?;
        let results: Vec<AuditLogDoc> = cursor
            .try_collect()
            .await
            .map_err(|_| ServerError::internal_error("Cursor decode failed"))?;
        Ok(results)
    }

    pub fn to_audit_log(&self) -> AuditLog {
        AuditLog {
            user_name: self.user_name.clone(),
            user_email: self.user_mail.clone(),
            timestamp: self.timestamp.try_to_rfc3339_string().unwrap(),
            action: self.action.clone(),
            details: self.details.clone(),
            device_id: self.device_id.clone().map(|id| id.to_string()),
        }
    }
}
