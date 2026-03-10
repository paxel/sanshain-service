use serde::Serialize;

#[derive(Clone, Debug)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
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
