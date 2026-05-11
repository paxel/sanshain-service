use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Bad Request: {0}")]
    BadRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden")]
    Forbidden,
    #[error("Internal Error: {0}")]
    Internal(String),
}

impl From<crate::domain::ports::RepositoryError> for AppError {
    fn from(e: crate::domain::ports::RepositoryError) -> Self {
        match e {
            crate::domain::ports::RepositoryError::NotFound => {
                AppError::NotFound("Not found".to_string())
            }
            crate::domain::ports::RepositoryError::Conflict => {
                AppError::Conflict("Conflict".to_string())
            }
            crate::domain::ports::RepositoryError::Internal(msg) => AppError::Internal(msg),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiType {
    #[default]
    OpenApi,
    AsyncApi,
    Proto,
}

impl ApiType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiType::OpenApi => "openapi",
            ApiType::AsyncApi => "asyncapi",
            ApiType::Proto => "proto",
        }
    }
}

impl std::str::FromStr for ApiType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openapi" | "rest" => Ok(ApiType::OpenApi),
            "asyncapi" | "kafka" | "async" => Ok(ApiType::AsyncApi),
            "proto" | "grpc" => Ok(ApiType::Proto),
            _ => Err(format!("Unknown API type: {}", s)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProvideResponse {
    pub version: i32,
    pub content_hash: String,
    pub changes: ProvideChanges,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProvideChanges {
    pub inserts: usize,
    pub updates: usize,
    pub deletes: usize,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    pub username: String,
    pub is_admin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub approved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub user_id: i64,
    pub expires_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EndpointRecord {
    pub id: Option<i64>,
    pub api_type: ApiType,
    pub path: String,
    pub normalized_path: String,
    pub method: String,
    pub yaml_content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedContract {
    pub branch_name: String,
    pub service_id: i64,
    pub api_type: ApiType,
    pub path: String,
    pub method: String,
    pub source_yaml: String,
    pub current_yaml: String,
    pub owner_service_id: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NodeSource {
    Branch,
    Target,
    Both,
}

#[derive(Serialize, Clone, Debug)]
pub struct MergedDependencyInfo {
    pub api_type: ApiType,
    pub client: String,
    pub service: String,
    pub path: String,
    pub method: String,
    pub source: NodeSource,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConflictInfo {
    pub service: String,
    pub api_type: ApiType,
    pub path: String,
    pub method: String,
    pub description: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct MergedDependencyReport {
    pub branch: String,
    pub target: String,
    pub dependency_graph: Vec<MergedDependencyInfo>,
    pub unused_endpoints: Vec<EndpointInfo>,
    pub missing_endpoints: Vec<MissingEndpointInfo>,
    pub conflicts: Vec<ConflictInfo>,
    pub service_tags: HashMap<String, Vec<String>>,
    pub node_sources: HashMap<String, NodeSource>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyReport {
    pub branch: String,
    pub unused_endpoints: Vec<EndpointInfo>,
    pub missing_endpoints: Vec<MissingEndpointInfo>,
    pub dependency_graph: Vec<DependencyInfo>,
    #[serde(default)]
    pub service_tags: HashMap<String, Vec<String>>,
}

#[derive(Serialize, Clone, Debug)]
pub struct EndpointInfo {
    pub api_type: ApiType,
    pub service: String,
    pub path: String,
    pub method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct MissingEndpointInfo {
    pub api_type: ApiType,
    pub client: String,
    pub service: String,
    pub path: String,
    pub method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyInfo {
    pub api_type: ApiType,
    pub client: String,
    pub service: String,
    pub path: String,
    pub method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ClientEndpointInfo {
    pub api_type: ApiType,
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
    pub branches: Vec<String>,
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
        api_type: ApiType,
        path: String,
        normalized_path: String,
        method: String,
        yaml_content: String,
    },
    Update {
        api_type: ApiType,
        path: String,
        normalized_path: String,
        method: String,
        yaml_content: String,
    },
    Delete {
        api_type: ApiType,
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

#[derive(Serialize, Clone, Debug)]
pub struct LogResponse {
    pub errors: Vec<LogEntry>,
    pub warnings: Vec<LogEntry>,
    pub infos: Vec<LogEntry>,
    pub debugs: Vec<LogEntry>,
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

#[derive(Serialize, Clone, Debug)]
pub struct CacheStats {
    pub enabled: bool,
    pub memory_limit_mb: u64,
    pub estimated_memory_used_bytes: u64,
    pub entry_count: u64,
    pub hit_count: u64,
    pub miss_count: u64,
    pub hit_rate_percent: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn api_type_accepts_canonical_names_and_legacy_aliases() {
        let cases = [
            ("openapi", ApiType::OpenApi),
            ("REST", ApiType::OpenApi),
            ("asyncapi", ApiType::AsyncApi),
            ("kafka", ApiType::AsyncApi),
            ("async", ApiType::AsyncApi),
            ("proto", ApiType::Proto),
            ("grpc", ApiType::Proto),
        ];

        for (input, expected) in cases {
            assert_eq!(ApiType::from_str(input).unwrap(), expected);
        }
    }

    #[test]
    fn api_type_rejects_unknown_values() {
        assert_eq!(
            ApiType::from_str("soap").unwrap_err(),
            "Unknown API type: soap"
        );
    }

    #[test]
    fn api_type_as_str_returns_stable_wire_values() {
        assert_eq!(ApiType::OpenApi.as_str(), "openapi");
        assert_eq!(ApiType::AsyncApi.as_str(), "asyncapi");
        assert_eq!(ApiType::Proto.as_str(), "proto");
    }

    #[test]
    fn auth_mode_parses_and_formats_supported_modes() {
        let cases = [
            ("dev", AuthMode::Dev),
            ("local", AuthMode::Local),
            ("ldap", AuthMode::Ldap),
        ];

        for (input, expected) in cases {
            let parsed = AuthMode::from_str(input).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), input);
        }
        assert!(AuthMode::from_str("oauth").is_err());
    }

    #[test]
    fn app_error_maps_repository_errors_at_the_boundary() {
        let not_found: AppError = crate::domain::ports::RepositoryError::NotFound.into();
        let conflict: AppError = crate::domain::ports::RepositoryError::Conflict.into();
        let internal: AppError =
            crate::domain::ports::RepositoryError::Internal("boom".into()).into();

        assert!(matches!(not_found, AppError::NotFound(message) if message == "Not found"));
        assert!(matches!(conflict, AppError::Conflict(message) if message == "Conflict"));
        assert!(matches!(internal, AppError::Internal(message) if message == "boom"));
    }

    #[test]
    fn ldap_config_defaults_to_uid_user_filter() {
        let config: LdapConfig = serde_json::from_value(serde_json::json!({
            "server_url": "ldap://example.test",
            "bind_dn": "cn=admin,dc=example,dc=test",
            "base_dn": "dc=example,dc=test"
        }))
        .unwrap();

        assert_eq!(config.user_filter, "(uid={username})");
        assert!(config.group_filter.is_empty());
        assert!(config.admin_group.is_empty());
        assert!(!config.use_tls);
    }

    #[test]
    fn ldap_config_serializes_password_for_persistence() {
        let serialized = serde_json::to_value(LdapConfig {
            server_url: "ldap://example.test".into(),
            bind_dn: "cn=admin,dc=example,dc=test".into(),
            bind_password: Some("secret".into()),
            base_dn: "dc=example,dc=test".into(),
            user_filter: "(uid={username})".into(),
            group_filter: String::new(),
            admin_group: String::new(),
            use_tls: false,
        })
        .unwrap();

        assert_eq!(serialized["bind_password"], "secret");
    }

    #[test]
    fn ldap_config_validation_accepts_ldap_and_ldaps_hosts() {
        for server_url in ["ldap://example.test", "ldaps://example.test"] {
            let config = LdapConfig {
                server_url: server_url.into(),
                bind_dn: "cn=admin,dc=example,dc=test".into(),
                bind_password: None,
                base_dn: "dc=example,dc=test".into(),
                user_filter: "(uid={username})".into(),
                group_filter: String::new(),
                admin_group: String::new(),
                use_tls: false,
            };

            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn ldap_config_validation_rejects_invalid_required_fields() {
        let valid = LdapConfig {
            server_url: "ldap://example.test".into(),
            bind_dn: "cn=admin,dc=example,dc=test".into(),
            bind_password: None,
            base_dn: "dc=example,dc=test".into(),
            user_filter: "(uid={username})".into(),
            group_filter: String::new(),
            admin_group: String::new(),
            use_tls: false,
        };

        let mut config = valid.clone();
        config.server_url.clear();
        assert_eq!(
            config.validate().unwrap_err(),
            "Server URL must not be empty"
        );

        let mut config = valid.clone();
        config.server_url = "https://example.test".into();
        assert_eq!(
            config.validate().unwrap_err(),
            "Server URL must start with ldap:// or ldaps://"
        );

        let mut config = valid.clone();
        config.server_url = "ldap://bad host".into();
        assert_eq!(config.validate().unwrap_err(), "Invalid LDAP server host");

        let mut config = valid.clone();
        config.server_url = "ldap:///bad".into();
        assert_eq!(config.validate().unwrap_err(), "Invalid LDAP server host");

        let mut config = valid.clone();
        config.bind_dn.clear();
        assert_eq!(config.validate().unwrap_err(), "Bind DN must not be empty");

        let mut config = valid;
        config.base_dn.clear();
        assert_eq!(config.validate().unwrap_err(), "Base DN must not be empty");
    }
}
