use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AuthMode {
    Dev,
    Local,
    Ldap,
}

impl AuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMode::Dev => "dev",
            AuthMode::Local => "local",
            AuthMode::Ldap => "ldap",
        }
    }
}

impl std::str::FromStr for AuthMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dev" => Ok(AuthMode::Dev),
            "local" => Ok(AuthMode::Local),
            "ldap" => Ok(AuthMode::Ldap),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LdapConfig {
    pub server_url: String,
    pub bind_dn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_password: Option<String>,
    pub base_dn: String,
    #[serde(default = "LdapConfig::default_user_filter")]
    pub user_filter: String,
    #[serde(default)]
    pub group_filter: String,
    #[serde(default)]
    pub admin_group: String,
    #[serde(default)]
    pub use_tls: bool,
}

impl LdapConfig {
    fn default_user_filter() -> String {
        "(uid={username})".to_string()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.server_url.is_empty() {
            return Err("Server URL must not be empty".to_string());
        }
        if !self.server_url.starts_with("ldap://") && !self.server_url.starts_with("ldaps://") {
            return Err("Server URL must start with ldap:// or ldaps://".to_string());
        }
        let host_part = if self.server_url.starts_with("ldap://") {
            &self.server_url[7..]
        } else {
            &self.server_url[8..]
        };
        if host_part.is_empty() || host_part.contains(' ') || host_part.starts_with('/') {
            return Err("Invalid LDAP server host".to_string());
        }
        if self.bind_dn.is_empty() {
            return Err("Bind DN must not be empty".to_string());
        }
        if self.base_dn.is_empty() {
            return Err("Base DN must not be empty".to_string());
        }
        Ok(())
    }
}

/// Represents an authenticated user from any auth provider.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub username: String,
    pub is_admin: bool,
}

#[derive(Clone, Debug)]
pub struct ApiToken {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub approved: bool,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub token: String,
    pub user_id: i64,
    pub expires_at: String,
}

#[derive(Clone, Debug)]
pub struct EndpointRecord {
    pub id: Option<i64>,
    pub path: String,
    pub normalized_path: String,
    pub method: String,
    pub yaml_content: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyReport {
    pub branch: String,
    pub unused_endpoints: Vec<EndpointInfo>,
    pub missing_endpoints: Vec<MissingEndpointInfo>,
    pub dependency_graph: Vec<DependencyInfo>,
}

#[derive(Serialize, Clone, Debug)]
pub struct EndpointInfo {
    pub service: String,
    pub path: String,
    pub method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct MissingEndpointInfo {
    pub client: String,
    pub service: String,
    pub path: String,
    pub method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyInfo {
    pub client: String,
    pub service: String,
    pub path: String,
    pub method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ClientEndpointInfo {
    pub service: String,
    pub branch: String,
    pub path: String,
    pub method: String,
    pub yaml_content: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ServiceSummary {
    pub name: String,
    pub fallback_branch: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct EndpointVersion {
    pub id: i64,
    pub endpoint_id: i64,
    pub version: i32,
    pub yaml_content: String,
    pub diff_from_previous: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub enum SpecChange {
    Insert {
        path: String,
        normalized_path: String,
        method: String,
        yaml_content: String,
    },
    Update {
        path: String,
        normalized_path: String,
        method: String,
        yaml_content: String,
    },
    Delete {
        path: String,
        method: String,
        soft_delete: bool,
    },
}

#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct SystemStats {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub system_uptime: u64,
    pub process_uptime: u64,
    pub requests_total: u64,
    pub failures_total: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DebugConfig {
    pub business_logic_debug: bool,
    pub admin_user_debug: bool,
}
