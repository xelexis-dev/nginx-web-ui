//! Role → action matrix, identical to `packages/auth/src/rbac.ts`.
//!
//! Purpose-bound token scopes (`node:pair`) are deliberately NOT actions: they never pass
//! through this matrix, so a user token can never gain them through a role.

use serde::{Deserialize, Serialize};

use crate::contracts::Role;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    #[serde(rename = "config:read")]
    ConfigRead,
    #[serde(rename = "config:write")]
    ConfigWrite,
    #[serde(rename = "user:manage")]
    UserManage,
    #[serde(rename = "nginx:reload")]
    NginxReload,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::ConfigRead, Action::ConfigWrite, Action::UserManage, Action::NginxReload];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::ConfigRead => "config:read",
            Action::ConfigWrite => "config:write",
            Action::UserManage => "user:manage",
            Action::NginxReload => "nginx:reload",
        }
    }
}

pub fn can(role: Role, action: Action) -> bool {
    match role {
        Role::Viewer => matches!(action, Action::ConfigRead),
        Role::Editor => matches!(action, Action::ConfigRead | Action::ConfigWrite | Action::NginxReload),
        Role::Admin => true,
    }
}

pub fn role_str(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Editor => "editor",
        Role::Viewer => "viewer",
    }
}

/// The `role` enum column as text (`role::text` in SQL); `None` for anything else.
pub fn parse_role(value: &str) -> Option<Role> {
    match value {
        "admin" => Some(Role::Admin),
        "editor" => Some(Role::Editor),
        "viewer" => Some(Role::Viewer),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The TS MATRIX, spelled out: viewer reads; editor also writes and reloads; admin also
    /// manages users.
    #[test]
    fn matrix_matches_the_ts_table() {
        use Action::*;
        let expected: [(Role, &[Action]); 3] = [
            (Role::Viewer, &[ConfigRead]),
            (Role::Editor, &[ConfigRead, ConfigWrite, NginxReload]),
            (Role::Admin, &[ConfigRead, ConfigWrite, NginxReload, UserManage]),
        ];
        for (role, allowed) in expected {
            for action in Action::ALL {
                assert_eq!(can(role, action), allowed.contains(&action), "{role:?} {action:?}");
            }
        }
    }

    #[test]
    fn wire_names_match_the_ts_vocabulary() {
        let names: Vec<&str> = Action::ALL.iter().map(|a| a.as_str()).collect();
        assert_eq!(names, ["config:read", "config:write", "user:manage", "nginx:reload"]);
        for action in Action::ALL {
            assert_eq!(serde_json::to_value(action).unwrap(), action.as_str());
        }
        assert_eq!(role_str(Role::Admin), "admin");
        assert_eq!(role_str(Role::Editor), "editor");
        assert_eq!(role_str(Role::Viewer), "viewer");
    }
}
