use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Editor,
    Viewer,
}

impl Default for Role {
    fn default() -> Self {
        Role::Viewer
    }
}

impl Role {
    pub fn to_string(&self) -> String {
        match self {
            Role::Owner => "owner".to_string(),
            Role::Admin => "admin".to_string(),
            Role::Editor => "editor".to_string(),
            Role::Viewer => "viewer".to_string(),
        }
    }

    pub fn rank(&self) -> u8 {
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

    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "owner" => Ok(Role::Owner),
            "admin" => Ok(Role::Admin),
            "editor" => Ok(Role::Editor),
            "viewer" => Ok(Role::Viewer),
            _ => Err(format!(
                "Invalid role: {}. Choose from owner, admin, editor, viewer",
                s
            )),
        }
    }
}
