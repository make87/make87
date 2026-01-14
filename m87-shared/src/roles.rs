use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Editor,
    Viewer,
}

impl Role {
    fn rank(&self) -> u8 {
        match self {
            Role::Viewer => 0,
            Role::Editor => 1,
            Role::Admin => 2,
            Role::Owner => 3,
        }
    }

    pub fn allows(have: &Role, need: &Role) -> bool {
        have.rank() >= need.rank()
    }
}
