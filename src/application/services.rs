use std::collections::HashMap;

use crate::domain::models::*;
use crate::domain::ports::{RepositoryError, SpecRepository};
use crate::openapi;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Conflict,
    NotFound,
    Unauthorized,
    Forbidden,
    Internal(String),
}

impl From<RepositoryError> for AppError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::NotFound => AppError::NotFound,
            RepositoryError::Conflict => AppError::Conflict,
            RepositoryError::Internal(msg) => AppError::Internal(msg),
        }
    }
}

pub async fn provide_spec(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    openapi_yaml: &str,
) -> Result<(), AppError> {
    let endpoints = openapi::split_openapi(openapi_yaml)
        .map_err(|e| AppError::BadRequest(e))?;

    let service_id = repo.ensure_service(servicename).await?;
    let branch_id = repo.ensure_branch(service_id, branch).await?;
    let is_protected = repo.is_branch_protected(branch).await?;

    let existing = repo.get_endpoints_for_branch(branch_id).await?;
    let mut existing_map: HashMap<(String, String), String> = existing
        .into_iter()
        .map(|e| ((e.path, e.method), e.yaml_content))
        .collect();

    let mut to_insert = Vec::new();
    let mut to_update = Vec::new();

    for endpoint in endpoints {
        let key = (endpoint.path.clone(), endpoint.method.clone());
        if let Some(existing_yaml) = existing_map.remove(&key) {
            if existing_yaml != endpoint.yaml_content {
                if is_protected {
                    tracing::warn!(
                        "Rejected update for {} {}: DTO changed on protected branch",
                        endpoint.method,
                        endpoint.path
                    );
                    return Err(AppError::Conflict);
                } else {
                    tracing::info!(
                        "Updating {} {} on feature branch",
                        endpoint.method,
                        endpoint.path
                    );
                    to_update.push(endpoint);
                }
            }
        } else {
            to_insert.push(EndpointRecord {
                id: None,
                path: endpoint.path,
                method: endpoint.method,
                yaml_content: endpoint.yaml_content,
            });
        }
    }

    for endpoint in &to_insert {
        repo.insert_endpoint(branch_id, endpoint).await?;
    }

    for endpoint in &to_update {
        repo.update_endpoint(branch_id, &endpoint.path, &endpoint.method, &endpoint.yaml_content).await?;
    }

    Ok(())
}

pub async fn require_endpoint(
    repo: &impl SpecRepository,
    clientname: &str,
    servicename: &str,
    branch: &str,
    path: &str,
    method: &str,
    timeout_secs: Option<u64>,
) -> Result<String, AppError> {
    let client_id = repo.ensure_client(clientname).await?;
    let service_id = repo.ensure_service(servicename).await?;

    let method_upper = method.to_uppercase();

    let deadline = timeout_secs.map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        let endpoint = repo.find_endpoint(service_id, branch, path, &method_upper).await?;

        // Feature branch fallback: if not found on a non-protected branch, try protected branches
        let endpoint = if endpoint.is_none() && !repo.is_branch_protected(branch).await? {
            let protected = repo.list_protected_branches().await?;
            let mut fallback = None;
            for pb in &protected {
                fallback = repo.find_endpoint(service_id, pb, path, &method_upper).await?;
                if fallback.is_some() {
                    tracing::info!(
                        "Falling back to protected branch '{}' for {} {}",
                        pb, method_upper, path
                    );
                    break;
                }
            }
            fallback
        } else {
            endpoint
        };

        if let Some(ref ep) = endpoint {
            let endpoint_id = Some(ep.0);
            repo.record_dependency(client_id, endpoint_id, service_id, branch, path, &method_upper).await?;
            return Ok(ep.1.clone());
        }

        // If no timeout or deadline passed, return NotFound
        match deadline {
            Some(dl) if std::time::Instant::now() < dl => {
                tokio::time::sleep(poll_interval).await;
            }
            _ => {
                repo.record_dependency(client_id, None, service_id, branch, path, &method_upper).await?;
                return Err(AppError::NotFound);
            }
        }
    }
}

pub async fn generate_report(
    repo: &impl SpecRepository,
    branch: &str,
) -> Result<DependencyReport, AppError> {
    Ok(repo.get_report(branch).await?)
}

pub async fn list_protected_branches(
    repo: &impl SpecRepository,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_protected_branches().await?)
}

pub async fn add_protected_branch(
    repo: &impl SpecRepository,
    pattern: &str,
) -> Result<(), AppError> {
    repo.add_protected_branch(pattern).await?;
    Ok(())
}

pub async fn remove_protected_branch(
    repo: &impl SpecRepository,
    pattern: &str,
) -> Result<bool, AppError> {
    Ok(repo.remove_protected_branch(pattern).await?)
}

pub async fn delete_service(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<bool, AppError> {
    Ok(repo.delete_service(name).await?)
}

pub async fn delete_branch(
    repo: &impl SpecRepository,
    service_name: &str,
    branch_name: &str,
) -> Result<bool, AppError> {
    Ok(repo.delete_branch(service_name, branch_name).await?)
}

pub async fn delete_client(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<bool, AppError> {
    Ok(repo.delete_client(name).await?)
}

pub async fn list_services(
    repo: &impl SpecRepository,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_services().await?)
}

pub async fn list_branches(
    repo: &impl SpecRepository,
    service_name: &str,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_branches(service_name).await?)
}

pub async fn list_clients(
    repo: &impl SpecRepository,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_clients().await?)
}

pub async fn list_client_branches(
    repo: &impl SpecRepository,
    client_name: &str,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_client_branches(client_name).await?)
}

pub async fn list_client_endpoints(
    repo: &impl SpecRepository,
    client_name: &str,
    branch: &str,
) -> Result<Vec<ClientEndpointInfo>, AppError> {
    Ok(repo.list_client_endpoints(client_name, branch).await?)
}

// --- Auth ---

pub fn hash_password(password: &str) -> Result<String, AppError> {
    use argon2::{Argon2, PasswordHasher};

    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("password hash error: {}", e)))
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    use argon2::{Argon2, PasswordVerifier};
    use argon2::password_hash::phc::PasswordHash;

    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("invalid hash: {}", e)))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

pub fn generate_random_password() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let chars: Vec<char> = (0..24)
        .map(|_| {
            let idx = rng.random_range(0..62);
            match idx {
                0..=9 => (b'0' + idx) as char,
                10..=35 => (b'a' + idx - 10) as char,
                _ => (b'A' + idx - 36) as char,
            }
        })
        .collect();
    chars.into_iter().collect()
}

pub async fn ensure_initial_admin(repo: &impl SpecRepository) -> Result<(), AppError> {
    let count = repo.user_count().await?;
    if count == 0 {
        let password = generate_random_password();
        let password_hash = hash_password(&password)?;
        let user = repo.create_user("root", &password_hash, true, true).await?;

        let expires_at = "2099-12-31T23:59:59";
        let session = repo.create_session(user.id, expires_at).await?;

        eprintln!("════════════════════════════════════════════════════");
        eprintln!("  INITIAL ROOT USER CREATED");
        eprintln!("  Username: root");
        eprintln!("  Password: {}", password);
        eprintln!("  Token:    {}", session.token);
        eprintln!("════════════════════════════════════════════════════");
        eprintln!("  Change the password immediately at:");
        eprintln!("  http://localhost:3000/admin.html");
        eprintln!("════════════════════════════════════════════════════");
    }
    Ok(())
}

pub async fn login(
    repo: &impl SpecRepository,
    username: &str,
    password: &str,
) -> Result<Session, AppError> {
    let user = repo.find_user(username).await?
        .ok_or(AppError::Unauthorized)?;

    if !verify_password(password, &user.password_hash)? {
        return Err(AppError::Unauthorized);
    }

    if !user.approved {
        return Err(AppError::Forbidden);
    }

    // Session expires in 24 hours
    let expires = chrono_expires_24h();
    let session = repo.create_session(user.id, &expires).await?;
    Ok(session)
}

pub async fn change_password(
    repo: &impl SpecRepository,
    user: &User,
    old_password: &str,
    new_password: &str,
) -> Result<(), AppError> {
    if !verify_password(old_password, &user.password_hash)? {
        return Err(AppError::Unauthorized);
    }
    let new_hash = hash_password(new_password)?;
    repo.update_password(user.id, &new_hash).await?;
    Ok(())
}

pub async fn get_dev_mode(repo: &impl SpecRepository) -> Result<bool, AppError> {
    let val = repo.get_setting("dev_mode").await?;
    Ok(val.as_deref() == Some("true"))
}

pub async fn set_dev_mode(repo: &impl SpecRepository, enabled: bool) -> Result<(), AppError> {
    repo.set_setting("dev_mode", if enabled { "true" } else { "false" }).await?;
    Ok(())
}

pub async fn validate_session(
    repo: &impl SpecRepository,
    token: &str,
) -> Result<Option<(User, Session)>, AppError> {
    Ok(repo.validate_session(token).await?)
}

pub async fn logout(
    repo: &impl SpecRepository,
    token: &str,
) -> Result<(), AppError> {
    repo.delete_session(token).await?;
    Ok(())
}

pub async fn register_user(
    repo: &impl SpecRepository,
    username: &str,
    password: &str,
) -> Result<(), AppError> {
    // Check if local users are enabled
    let enabled = repo.get_setting("local_users_enabled").await?;
    if enabled.as_deref() != Some("true") {
        return Err(AppError::Forbidden);
    }

    if username.is_empty() || password.is_empty() {
        return Err(AppError::BadRequest("Username and password must not be empty".to_string()));
    }

    // Check if username already exists
    if repo.find_user(username).await?.is_some() {
        return Err(AppError::Conflict);
    }

    let password_hash = hash_password(password)?;
    repo.create_user(username, &password_hash, false, false).await?;
    Ok(())
}

pub async fn list_users(
    repo: &impl SpecRepository,
) -> Result<Vec<User>, AppError> {
    Ok(repo.list_users().await?)
}

pub async fn approve_user(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(repo.approve_user(user_id).await?)
}

pub async fn admin_delete_user(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(repo.delete_user(user_id).await?)
}

pub async fn get_local_users_enabled(repo: &impl SpecRepository) -> Result<bool, AppError> {
    let val = repo.get_setting("local_users_enabled").await?;
    Ok(val.as_deref() == Some("true"))
}

pub async fn set_local_users_enabled(repo: &impl SpecRepository, enabled: bool) -> Result<(), AppError> {
    repo.set_setting("local_users_enabled", if enabled { "true" } else { "false" }).await?;
    Ok(())
}

// --- API Token Management ---

/// Create a new API token. Returns (token_id, raw_token) — the raw token is shown only once.
pub async fn create_api_token(
    repo: &impl SpecRepository,
    user_id: i64,
    name: &str,
    expires_in_days: u64,
) -> Result<(String, String), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::BadRequest("Token name must not be empty".into()));
    }
    if expires_in_days == 0 || expires_in_days > 3650 {
        return Err(AppError::BadRequest("Expiry must be between 1 and 3650 days".into()));
    }

    // Generate random token with san_ prefix
    use rand::Rng;
    let mut token_bytes = [0u8; 32];
    rand::rng().fill(&mut token_bytes);
    let raw_token = format!("san_{}", hex::encode(token_bytes));

    // SHA-256 hash for storage
    use sha2::{Sha256, Digest};
    let token_hash = hex::encode(Sha256::digest(raw_token.as_bytes()));

    // Generate ID
    let mut id_bytes = [0u8; 16];
    rand::rng().fill(&mut id_bytes);
    let id = hex::encode(id_bytes);

    let now = current_utc_iso();
    let expires_at = future_utc_iso(expires_in_days * 86400);

    repo.create_api_token(&id, user_id, name.trim(), &token_hash, &now, &expires_at).await?;

    Ok((id, raw_token))
}

pub async fn list_api_tokens(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<Vec<ApiToken>, AppError> {
    Ok(repo.list_api_tokens(user_id).await?)
}

pub async fn revoke_api_token(
    repo: &impl SpecRepository,
    token_id: &str,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(repo.delete_api_token(token_id, user_id).await?)
}

/// Validate a Bearer token that starts with "san_". Returns the user if valid.
pub async fn validate_api_token(
    repo: &impl SpecRepository,
    raw_token: &str,
) -> Result<Option<User>, AppError> {
    use sha2::{Sha256, Digest};
    let token_hash = hex::encode(Sha256::digest(raw_token.as_bytes()));
    Ok(repo.validate_api_token(&token_hash).await?)
}

fn current_utc_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    secs_to_iso(now)
}

fn future_utc_iso(offset_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    secs_to_iso(now + offset_secs)
}

fn secs_to_iso(secs: u64) -> String {
    let secs_per_day = 86400u64;
    let days_since_epoch = secs / secs_per_day;
    let time_of_day = secs % secs_per_day;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", year, month, day, hours, minutes, seconds)
}

fn chrono_expires_24h() -> String {
    // Simple: current UTC + 24h as ISO string
    // We avoid adding chrono dep by computing manually
    // Use a fixed format that SQLite datetime() understands
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expires = now + 86400;
    // Format as ISO 8601
    let secs_per_day = 86400u64;
    let days_since_epoch = expires / secs_per_day;
    let time_of_day = expires % secs_per_day;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Simple date calculation from days since epoch
    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", year, month, day, hours, minutes, seconds)
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn render_report_markdown(report: &DependencyReport) -> String {
    let mut md = format!("# Sanshain Dependency Report: Branch `{}`\n\n", report.branch);

    md.push_str("## Summary\n");
    md.push_str(&format!("- Total Dependencies: {}\n", report.dependency_graph.len()));
    md.push_str(&format!("- Unused Endpoints: {}\n", report.unused_endpoints.len()));
    md.push_str(&format!("- Missing Requirements: {}\n\n", report.missing_endpoints.len()));

    md.push_str("## Dependency Graph\n");
    if report.dependency_graph.is_empty() {
        md.push_str("No active dependencies recorded for this branch.\n\n");
    } else {
        md.push_str("| Client | Service | Path | Method |\n");
        md.push_str("| --- | --- | --- | --- |\n");
        for dep in &report.dependency_graph {
            md.push_str(&format!("| {} | {} | `{}` | `{}` |\n", dep.client, dep.service, dep.path, dep.method));
        }
        md.push_str("\n");
    }

    md.push_str("## Unused Endpoints\n");
    md.push_str("> Endpoints that are provided by a service but have no recorded client requirements.\n\n");
    if report.unused_endpoints.is_empty() {
        md.push_str("All provided endpoints are in use.\n\n");
    } else {
        md.push_str("| Service | Path | Method |\n");
        md.push_str("| --- | --- | --- |\n");
        for ep in &report.unused_endpoints {
            md.push_str(&format!("| {} | `{}` | `{}` |\n", ep.service, ep.path, ep.method));
        }
        md.push_str("\n");
    }

    md.push_str("## Missing Requirements\n");
    md.push_str("> Requirements from clients for endpoints that do not exist in this branch.\n\n");
    if report.missing_endpoints.is_empty() {
        md.push_str("No missing requirements identified.\n\n");
    } else {
        md.push_str("| Client | Service | Path | Method |\n");
        md.push_str("| --- | --- | --- | --- |\n");
        for ep in &report.missing_endpoints {
            md.push_str(&format!("| {} | {} | `{}` | `{}` |\n", ep.client, ep.service, ep.path, ep.method));
        }
        md.push_str("\n");
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::RepositoryError;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockRepo {
        services: Mutex<HashMap<String, i64>>,
        branches: Mutex<HashMap<(i64, String), i64>>,
        endpoints: Mutex<HashMap<i64, Vec<EndpointRecord>>>,
        clients: Mutex<HashMap<String, i64>>,
        protected_branches: Mutex<Vec<String>>,
        next_id: Mutex<i64>,
        users: Mutex<Vec<User>>,
        sessions: Mutex<Vec<Session>>,
        settings: Mutex<HashMap<String, String>>,
        api_tokens: Mutex<Vec<ApiToken>>,
    }

    impl MockRepo {
        fn new() -> Self {
            let mut settings = HashMap::new();
            settings.insert("dev_mode".to_string(), "false".to_string());
            Self {
                services: Mutex::new(HashMap::new()),
                branches: Mutex::new(HashMap::new()),
                endpoints: Mutex::new(HashMap::new()),
                clients: Mutex::new(HashMap::new()),
                protected_branches: Mutex::new(vec!["main".to_string(), "master".to_string()]),
                next_id: Mutex::new(1),
                users: Mutex::new(Vec::new()),
                sessions: Mutex::new(Vec::new()),
                settings: Mutex::new(settings),
                api_tokens: Mutex::new(Vec::new()),
            }
        }

        fn next_id(&self) -> i64 {
            let mut id = self.next_id.lock().unwrap();
            let current = *id;
            *id += 1;
            current
        }
    }

    impl SpecRepository for MockRepo {
        async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
            let mut services = self.services.lock().unwrap();
            if let Some(&id) = services.get(name) {
                return Ok(id);
            }
            let id = self.next_id();
            services.insert(name.to_string(), id);
            Ok(id)
        }

        async fn ensure_branch(&self, service_id: i64, branch_name: &str) -> Result<i64, RepositoryError> {
            let mut branches = self.branches.lock().unwrap();
            let key = (service_id, branch_name.to_string());
            if let Some(&id) = branches.get(&key) {
                return Ok(id);
            }
            let id = self.next_id();
            branches.insert(key, id);
            Ok(id)
        }

        async fn get_endpoints_for_branch(&self, branch_id: i64) -> Result<Vec<EndpointRecord>, RepositoryError> {
            let endpoints = self.endpoints.lock().unwrap();
            Ok(endpoints.get(&branch_id).cloned().unwrap_or_default())
        }

        async fn insert_endpoint(&self, branch_id: i64, endpoint: &EndpointRecord) -> Result<(), RepositoryError> {
            let mut endpoints = self.endpoints.lock().unwrap();
            let mut record = endpoint.clone();
            record.id = Some(self.next_id());
            endpoints.entry(branch_id).or_default().push(record);
            Ok(())
        }

        async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
            let mut clients = self.clients.lock().unwrap();
            if let Some(&id) = clients.get(name) {
                return Ok(id);
            }
            let id = self.next_id();
            clients.insert(name.to_string(), id);
            Ok(id)
        }

        async fn find_endpoint(
            &self,
            service_id: i64,
            branch_name: &str,
            path: &str,
            method: &str,
        ) -> Result<Option<(i64, String)>, RepositoryError> {
            let branches = self.branches.lock().unwrap();
            let key = (service_id, branch_name.to_string());
            if let Some(&branch_id) = branches.get(&key) {
                let endpoints = self.endpoints.lock().unwrap();
                if let Some(eps) = endpoints.get(&branch_id) {
                    for ep in eps {
                        if ep.path == path && ep.method == method {
                            return Ok(Some((ep.id.unwrap(), ep.yaml_content.clone())));
                        }
                    }
                }
            }
            Ok(None)
        }

        async fn record_dependency(
            &self,
            _client_id: i64,
            _endpoint_id: Option<i64>,
            _service_id: i64,
            _branch_name: &str,
            _path: &str,
            _method: &str,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
            Ok(DependencyReport {
                branch: branch.to_string(),
                dependency_graph: vec![],
                unused_endpoints: vec![],
                missing_endpoints: vec![],
            })
        }

        async fn is_branch_protected(&self, branch_name: &str) -> Result<bool, RepositoryError> {
            let pb = self.protected_branches.lock().unwrap();
            Ok(pb.contains(&branch_name.to_string()))
        }

        async fn add_protected_branch(&self, pattern: &str) -> Result<(), RepositoryError> {
            let mut pb = self.protected_branches.lock().unwrap();
            if !pb.contains(&pattern.to_string()) {
                pb.push(pattern.to_string());
            }
            Ok(())
        }

        async fn remove_protected_branch(&self, pattern: &str) -> Result<bool, RepositoryError> {
            let mut pb = self.protected_branches.lock().unwrap();
            let len_before = pb.len();
            pb.retain(|p| p != pattern);
            Ok(pb.len() < len_before)
        }

        async fn list_protected_branches(&self) -> Result<Vec<String>, RepositoryError> {
            let pb = self.protected_branches.lock().unwrap();
            Ok(pb.clone())
        }

        async fn update_endpoint(&self, branch_id: i64, path: &str, method: &str, yaml_content: &str) -> Result<(), RepositoryError> {
            let mut endpoints = self.endpoints.lock().unwrap();
            if let Some(eps) = endpoints.get_mut(&branch_id) {
                for ep in eps.iter_mut() {
                    if ep.path == path && ep.method == method {
                        ep.yaml_content = yaml_content.to_string();
                        return Ok(());
                    }
                }
            }
            Ok(())
        }

        async fn delete_service(&self, name: &str) -> Result<bool, RepositoryError> {
            let mut services = self.services.lock().unwrap();
            if let Some(&service_id) = services.get(name) {
                services.remove(name);
                let mut branches = self.branches.lock().unwrap();
                let mut endpoints = self.endpoints.lock().unwrap();
                let branch_ids: Vec<i64> = branches.iter()
                    .filter(|((sid, _), _)| *sid == service_id)
                    .map(|(_, &bid)| bid)
                    .collect();
                branches.retain(|(sid, _), _| *sid != service_id);
                for bid in branch_ids {
                    endpoints.remove(&bid);
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }

        async fn delete_branch(&self, service_name: &str, branch_name: &str) -> Result<bool, RepositoryError> {
            let services = self.services.lock().unwrap();
            if let Some(&service_id) = services.get(service_name) {
                let mut branches = self.branches.lock().unwrap();
                let key = (service_id, branch_name.to_string());
                if let Some(&branch_id) = branches.get(&key) {
                    branches.remove(&key);
                    let mut endpoints = self.endpoints.lock().unwrap();
                    endpoints.remove(&branch_id);
                    Ok(true)
                } else {
                    Ok(false)
                }
            } else {
                Ok(false)
            }
        }

        async fn delete_client(&self, name: &str) -> Result<bool, RepositoryError> {
            let mut clients = self.clients.lock().unwrap();
            if clients.remove(name).is_some() {
                Ok(true)
            } else {
                Ok(false)
            }
        }

        async fn list_services(&self) -> Result<Vec<String>, RepositoryError> {
            let services = self.services.lock().unwrap();
            let mut names: Vec<String> = services.keys().cloned().collect();
            names.sort();
            Ok(names)
        }

        async fn list_branches(&self, service_name: &str) -> Result<Vec<String>, RepositoryError> {
            let services = self.services.lock().unwrap();
            if let Some(&service_id) = services.get(service_name) {
                let branches = self.branches.lock().unwrap();
                let mut names: Vec<String> = branches.iter()
                    .filter(|((sid, _), _)| *sid == service_id)
                    .map(|((_, name), _)| name.clone())
                    .collect();
                names.sort();
                Ok(names)
            } else {
                Ok(vec![])
            }
        }

        async fn list_clients(&self) -> Result<Vec<String>, RepositoryError> {
            let clients = self.clients.lock().unwrap();
            let mut names: Vec<String> = clients.keys().cloned().collect();
            names.sort();
            Ok(names)
        }

        async fn list_client_branches(&self, _client_name: &str) -> Result<Vec<String>, RepositoryError> {
            Ok(vec![])
        }

        async fn list_client_endpoints(&self, _client_name: &str, _branch: &str) -> Result<Vec<ClientEndpointInfo>, RepositoryError> {
            Ok(vec![])
        }

        async fn user_count(&self) -> Result<i64, RepositoryError> {
            Ok(self.users.lock().unwrap().len() as i64)
        }

        async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
            Ok(self.users.lock().unwrap().iter().find(|u| u.username == username).cloned())
        }

        async fn create_user(&self, username: &str, password_hash: &str, is_admin: bool, approved: bool) -> Result<User, RepositoryError> {
            let id = self.next_id();
            let user = User { id, username: username.to_string(), password_hash: password_hash.to_string(), is_admin, approved };
            self.users.lock().unwrap().push(user.clone());
            Ok(user)
        }

        async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
            Ok(self.users.lock().unwrap().clone())
        }

        async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
            let mut users = self.users.lock().unwrap();
            if let Some(u) = users.iter_mut().find(|u| u.id == user_id && !u.approved) {
                u.approved = true;
                Ok(true)
            } else {
                Ok(false)
            }
        }

        async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
            let mut users = self.users.lock().unwrap();
            let len_before = users.len();
            users.retain(|u| u.id != user_id);
            Ok(users.len() < len_before)
        }

        async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
            let mut users = self.users.lock().unwrap();
            if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
                u.password_hash = new_hash.to_string();
            }
            Ok(())
        }

        async fn create_session(&self, user_id: i64, expires_at: &str) -> Result<Session, RepositoryError> {
            let token = format!("mock-token-{}", self.next_id());
            let session = Session { token: token.clone(), user_id, expires_at: expires_at.to_string() };
            self.sessions.lock().unwrap().push(session.clone());
            Ok(session)
        }

        async fn validate_session(&self, token: &str) -> Result<Option<(User, Session)>, RepositoryError> {
            let sessions = self.sessions.lock().unwrap();
            if let Some(s) = sessions.iter().find(|s| s.token == token) {
                let users = self.users.lock().unwrap();
                if let Some(u) = users.iter().find(|u| u.id == s.user_id) {
                    return Ok(Some((u.clone(), s.clone())));
                }
            }
            Ok(None)
        }

        async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
            self.sessions.lock().unwrap().retain(|s| s.token != token);
            Ok(())
        }

        async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
            Ok(self.settings.lock().unwrap().get(key).cloned())
        }

        async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
            self.settings.lock().unwrap().insert(key.to_string(), value.to_string());
            Ok(())
        }

        async fn create_api_token(&self, id: &str, user_id: i64, name: &str, token_hash: &str, created_at: &str, expires_at: &str) -> Result<(), RepositoryError> {
            let mut tokens = self.api_tokens.lock().unwrap();
            if tokens.iter().any(|t| t.user_id == user_id && t.name == name) {
                return Err(RepositoryError::Conflict);
            }
            tokens.push(ApiToken {
                id: id.to_string(),
                user_id,
                name: name.to_string(),
                token_hash: token_hash.to_string(),
                created_at: created_at.to_string(),
                expires_at: expires_at.to_string(),
                last_used_at: None,
            });
            Ok(())
        }

        async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
            let tokens = self.api_tokens.lock().unwrap();
            Ok(tokens.iter().filter(|t| t.user_id == user_id).cloned().collect())
        }

        async fn delete_api_token(&self, token_id: &str, user_id: i64) -> Result<bool, RepositoryError> {
            let mut tokens = self.api_tokens.lock().unwrap();
            let len_before = tokens.len();
            tokens.retain(|t| !(t.id == token_id && t.user_id == user_id));
            Ok(tokens.len() < len_before)
        }

        async fn validate_api_token(&self, token_hash: &str) -> Result<Option<User>, RepositoryError> {
            let tokens = self.api_tokens.lock().unwrap();
            if let Some(t) = tokens.iter().find(|t| t.token_hash == token_hash) {
                let users = self.users.lock().unwrap();
                return Ok(users.iter().find(|u| u.id == t.user_id).cloned());
            }
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_provide_spec_success() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        let result = provide_spec(&repo, "svc", "main", yaml).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_provide_spec_idempotent() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        let result = provide_spec(&repo, "svc", "main", yaml).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_provide_spec_conflict_on_changed_dto() {
        let repo = MockRepo::new();
        let yaml1 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        let yaml2 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      description: Changed
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml1).await.unwrap();
        let result = provide_spec(&repo, "svc", "main", yaml2).await;
        assert!(matches!(result, Err(AppError::Conflict)));
    }

    #[tokio::test]
    async fn test_provide_spec_invalid_yaml() {
        let repo = MockRepo::new();
        let result = provide_spec(&repo, "svc", "main", "not valid [[[").await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn test_require_endpoint_not_found() {
        let repo = MockRepo::new();
        let result = require_endpoint(&repo, "client", "svc", "main", "/missing", "GET", None).await;
        assert!(matches!(result, Err(AppError::NotFound)));
    }

    #[tokio::test]
    async fn test_require_endpoint_found() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        let result = require_endpoint(&repo, "client", "svc", "main", "/users", "GET", None).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("/users"));
    }

    #[test]
    fn test_render_report_markdown_empty() {
        let report = DependencyReport {
            branch: "main".to_string(),
            dependency_graph: vec![],
            unused_endpoints: vec![],
            missing_endpoints: vec![],
        };
        let md = render_report_markdown(&report);
        assert!(md.contains("# Sanshain Dependency Report: Branch `main`"));
        assert!(md.contains("Total Dependencies: 0"));
        assert!(md.contains("No active dependencies recorded"));
    }

    #[test]
    fn test_render_report_markdown_with_data() {
        let report = DependencyReport {
            branch: "dev".to_string(),
            dependency_graph: vec![DependencyInfo {
                client: "web".to_string(),
                service: "api".to_string(),
                path: "/users".to_string(),
                method: "GET".to_string(),
            }],
            unused_endpoints: vec![EndpointInfo {
                service: "api".to_string(),
                path: "/old".to_string(),
                method: "DELETE".to_string(),
            }],
            missing_endpoints: vec![MissingEndpointInfo {
                client: "web".to_string(),
                service: "api".to_string(),
                path: "/new".to_string(),
                method: "POST".to_string(),
            }],
        };
        let md = render_report_markdown(&report);
        assert!(md.contains("| web | api | `/users` | `GET` |"));
        assert!(md.contains("| api | `/old` | `DELETE` |"));
        assert!(md.contains("| web | api | `/new` | `POST` |"));
    }

    #[tokio::test]
    async fn test_provide_spec_feature_branch_allows_update() {
        let repo = MockRepo::new();
        let yaml1 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        let yaml2 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      description: Changed
      responses:
        '200':
          description: OK
"#;
        // "feature/xyz" is not protected, so updates should be allowed
        provide_spec(&repo, "svc", "feature/xyz", yaml1).await.unwrap();
        let result = provide_spec(&repo, "svc", "feature/xyz", yaml2).await;
        assert!(result.is_ok());

        // Verify the endpoint was actually updated
        let content = require_endpoint(&repo, "client", "svc", "feature/xyz", "/users", "GET", None).await.unwrap();
        assert!(content.contains("Changed"));
    }

    #[tokio::test]
    async fn test_require_feature_branch_fallback_to_main() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        // Provide on main only
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();

        // Require on a feature branch that has no endpoints — should fallback to main
        let result = require_endpoint(&repo, "client", "svc", "feature/abc", "/users", "GET", None).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("/users"));
    }

    #[tokio::test]
    async fn test_require_with_timeout_returns_not_found_after_expiry() {
        let repo = MockRepo::new();
        let start = std::time::Instant::now();
        let result = require_endpoint(&repo, "client", "svc", "main", "/missing", "GET", Some(1)).await;
        let elapsed = start.elapsed();
        assert!(matches!(result, Err(AppError::NotFound)));
        assert!(elapsed >= std::time::Duration::from_millis(500), "should have polled at least once");
    }

    #[tokio::test]
    async fn test_require_no_fallback_on_protected_branch() {
        let repo = MockRepo::new();
        // Provide on main, require on master (also protected) — no fallback, should 404
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();

        // master is protected, so no fallback — endpoint not on master → NotFound
        let result = require_endpoint(&repo, "client", "svc", "master", "/users", "GET", None).await;
        assert!(matches!(result, Err(AppError::NotFound)));
    }

    #[tokio::test]
    async fn test_provide_spec_protected_branch_rejects_update() {
        let repo = MockRepo::new();
        let yaml1 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        let yaml2 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      description: Changed
      responses:
        '200':
          description: OK
"#;
        // "main" is protected by default
        provide_spec(&repo, "svc", "main", yaml1).await.unwrap();
        let result = provide_spec(&repo, "svc", "main", yaml2).await;
        assert!(matches!(result, Err(AppError::Conflict)));
    }

    #[tokio::test]
    async fn test_protected_branch_management() {
        let repo = MockRepo::new();

        let branches = list_protected_branches(&repo).await.unwrap();
        assert!(branches.contains(&"main".to_string()));
        assert!(branches.contains(&"master".to_string()));

        add_protected_branch(&repo, "release").await.unwrap();
        let branches = list_protected_branches(&repo).await.unwrap();
        assert!(branches.contains(&"release".to_string()));

        let removed = remove_protected_branch(&repo, "release").await.unwrap();
        assert!(removed);
        let branches = list_protected_branches(&repo).await.unwrap();
        assert!(!branches.contains(&"release".to_string()));

        let removed = remove_protected_branch(&repo, "nonexistent").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_delete_service() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        let services = list_services(&repo).await.unwrap();
        assert!(services.contains(&"svc".to_string()));

        let removed = delete_service(&repo, "svc").await.unwrap();
        assert!(removed);

        let services = list_services(&repo).await.unwrap();
        assert!(!services.contains(&"svc".to_string()));

        let removed = delete_service(&repo, "svc").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_delete_branch() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        provide_spec(&repo, "svc", "feature/x", yaml).await.unwrap();

        let branches = list_branches(&repo, "svc").await.unwrap();
        assert_eq!(branches.len(), 2);

        let removed = delete_branch(&repo, "svc", "feature/x").await.unwrap();
        assert!(removed);

        let branches = list_branches(&repo, "svc").await.unwrap();
        assert_eq!(branches.len(), 1);
        assert!(branches.contains(&"main".to_string()));

        let removed = delete_branch(&repo, "svc", "feature/x").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_delete_client() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        require_endpoint(&repo, "webclient", "svc", "main", "/users", "GET", None).await.unwrap();

        let clients = list_clients(&repo).await.unwrap();
        assert!(clients.contains(&"webclient".to_string()));

        let removed = delete_client(&repo, "webclient").await.unwrap();
        assert!(removed);

        let clients = list_clients(&repo).await.unwrap();
        assert!(!clients.contains(&"webclient".to_string()));

        let removed = delete_client(&repo, "webclient").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_create_and_list_api_tokens() {
        let repo = MockRepo::new();
        // Create a user first
        let user = repo.create_user("testuser", "hash", false, true).await.unwrap();

        // Create a token
        let (id, raw_token) = create_api_token(&repo, user.id, "jenkins-ci", 365).await.unwrap();
        assert!(raw_token.starts_with("san_"));
        assert!(!id.is_empty());

        // List tokens
        let tokens = list_api_tokens(&repo, user.id).await.unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].name, "jenkins-ci");

        // Duplicate name should fail
        let result = create_api_token(&repo, user.id, "jenkins-ci", 365).await;
        assert!(matches!(result, Err(AppError::Conflict)));
    }

    #[tokio::test]
    async fn test_revoke_api_token() {
        let repo = MockRepo::new();
        let user = repo.create_user("testuser", "hash", false, true).await.unwrap();

        let (id, _) = create_api_token(&repo, user.id, "my-token", 365).await.unwrap();

        let revoked = revoke_api_token(&repo, &id, user.id).await.unwrap();
        assert!(revoked);

        let tokens = list_api_tokens(&repo, user.id).await.unwrap();
        assert_eq!(tokens.len(), 0);

        // Revoking again should return false
        let revoked = revoke_api_token(&repo, &id, user.id).await.unwrap();
        assert!(!revoked);
    }

    #[tokio::test]
    async fn test_validate_api_token() {
        let repo = MockRepo::new();
        let user = repo.create_user("testuser", "hash", false, true).await.unwrap();

        let (_, raw_token) = create_api_token(&repo, user.id, "ci-token", 365).await.unwrap();

        // Validate the raw token
        let result = validate_api_token(&repo, &raw_token).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().username, "testuser");

        // Invalid token
        let result = validate_api_token(&repo, "san_invalid").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_create_api_token_validation() {
        let repo = MockRepo::new();
        let user = repo.create_user("testuser", "hash", false, true).await.unwrap();

        // Empty name
        let result = create_api_token(&repo, user.id, "", 365).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));

        // Zero days
        let result = create_api_token(&repo, user.id, "test", 0).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));

        // Too many days
        let result = create_api_token(&repo, user.id, "test", 5000).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
