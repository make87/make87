use serde::{Deserialize, Serialize};

use crate::roles::Role;
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct User {
    pub id: String,
    pub email: String,
    pub role: Role,
}
