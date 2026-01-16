use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::{StreamExt, TryStreamExt, stream};
use m87_shared::{device::PublicDevice, roles::Role, users::User};
use mongodb::bson::{doc, oid::ObjectId};
use tokio::time::timeout;

use crate::{
    db::Mongo,
    models::user::UserDoc,
    response::{ServerError, ServerResult},
};

pub async fn get_org_members(db: &Arc<Mongo>, org_ids: Vec<String>) -> ServerResult<Vec<User>> {
    let roles = db.roles().clone();
    let users = db.users().clone();

    // 1) Fetch org role maps concurrently, but bounded + timeout per org
    let mut emails: HashMap<String, Role> = HashMap::new();

    let maps: Vec<HashMap<String, Role>> = stream::iter(org_ids.into_iter())
        .map(|org_id| {
            let roles = roles.clone();
            async move {
                let org_scope = org_scope(&org_id);

                let fut = async {
                    let mut cursor = roles.find(doc! { "scope": &org_scope }).await?;
                    let mut map: HashMap<String, Role> = HashMap::new();

                    while let Some(role_doc) = cursor.try_next().await? {
                        if let Some(email) = role_doc.reference_id.strip_prefix("user:") {
                            // keep highest role if duplicates exist
                            match map.get(email) {
                                Some(existing) if existing.rank() >= role_doc.role.rank() => {}
                                _ => {
                                    map.insert(email.to_string(), role_doc.role);
                                }
                            }
                        }
                    }

                    Ok::<_, ServerError>(map)
                };

                // If a single org query stalls, fail that org with a clear error.
                match timeout(Duration::from_secs(10), fut).await {
                    Ok(res) => res,
                    Err(_) => Err(ServerError::timeout("org member lookup timed out")),
                }
            }
        })
        .buffer_unordered(16) // <= adjust concurrency
        .filter_map(|res| async {
            match res {
                Ok(map) => Some(map),
                Err(e) => {
                    // if you prefer "fail fast", replace with `return Err(e)` above and remove this.
                    eprintln!("Error loading org users: {e}");
                    None
                }
            }
        })
        .collect()
        .await;

    for map in maps {
        merge_roles(&mut emails, map); // use your rank-based merge
    }

    // 2) Resolve emails -> user docs
    if emails.is_empty() {
        return Ok(Vec::new());
    }

    let email_vec: Vec<String> = emails.keys().cloned().collect();
    let mut cursor = users.find(doc! { "email": { "$in": &email_vec } }).await?;

    let mut users_out = Vec::new();
    while let Some(udoc) = cursor.try_next().await? {
        if let Some(email) = &udoc.email {
            if let Some(role) = emails.get(email) {
                users_out.push(udoc.to_public_user(role));
            }
        }
    }

    Ok(users_out)
}

fn merge_roles(target: &mut HashMap<String, Role>, incoming: HashMap<String, Role>) {
    for (email, role) in incoming {
        target
            .entry(email)
            .and_modify(|existing| {
                if role.rank() > existing.rank() {
                    *existing = role.clone();
                }
            })
            .or_insert(role);
    }
}

pub async fn get_org_devices(
    db: &Arc<Mongo>,
    org_id: &str,
) -> Result<Vec<PublicDevice>, ServerError> {
    let mut cursor = db
        .roles()
        .find(doc! { "reference_id": org_ref(org_id), "scope": { "$regex": "^device:" } })
        .await?;

    let mut device_oids: Vec<ObjectId> = Vec::new();
    let mut device_role_map: HashMap<ObjectId, Role> = HashMap::new();
    while let Some(role_doc) = cursor.try_next().await? {
        if let Some(oid_str) = role_doc.scope.strip_prefix("device:") {
            if let Ok(oid) = ObjectId::parse_str(oid_str) {
                device_oids.push(oid);
                device_role_map.insert(oid, role_doc.role);
            }
        }
    }

    if device_oids.is_empty() {
        return Ok(vec![]);
    }

    let mut dcur = db
        .devices()
        .find(doc! { "_id": { "$in": &device_oids } })
        .await?;

    let mut out: Vec<PublicDevice> = Vec::new();
    while let Some(dev) = dcur.try_next().await? {
        let Some(role) = device_role_map.get(&dev.id.clone().unwrap()) else {
            continue;
        };
        // If you want the *org's* role on the device included, you'd need to fetch it and pass it here.
        out.push(dev.to_public_device(&role));
    }
    Ok(out)
}

pub async fn get_user_orgs(db: &Arc<Mongo>, user_mail: &str) -> Result<Vec<String>, ServerError> {
    let user_reference = UserDoc::create_reference_id(user_mail);
    let mut cursor = db
        .roles()
        .find(doc! { "reference_id": &user_reference, "scope": { "$regex": "^org:" } })
        .await?;

    let mut org_oids: Vec<String> = Vec::new();
    while let Some(role_doc) = cursor.try_next().await? {
        if let Some(oid_str) = role_doc.scope.strip_prefix("org:") {
            org_oids.push(oid_str.to_string());
        }
    }
    Ok(org_oids)
}

pub fn org_scope(org_id: &str) -> String {
    format!("org:{}", org_id)
}

pub fn org_ref(org_id: &str) -> String {
    org_scope(org_id)
}
