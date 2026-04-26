use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use chrono::Utc;

pub async fn list_protected_branches(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_protected_branches().await?)
}

pub async fn add_protected_branch(repo: &impl SpecRepository, pattern: &str) -> Result<(), AppError> {
    repo.add_protected_branch(pattern).await?;
    Ok(())
}

pub async fn remove_protected_branch(repo: &impl SpecRepository, pattern: &str) -> Result<bool, AppError> {
    Ok(repo.remove_protected_branch(pattern).await?)
}

pub async fn delete_all_services(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_all_services().await?)
}

pub async fn delete_all_clients(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_all_clients().await?)
}

pub async fn delete_all_non_admin_users(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_all_non_admin_users().await?)
}

pub async fn nuke_database(repo: &impl SpecRepository, keep_user_id: Option<i64>) -> Result<(), AppError> {
    repo.nuke_database(keep_user_id).await?;
    Ok(())
}

pub async fn delete_service(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    Ok(repo.delete_service(name).await?)
}

pub async fn delete_branch(repo: &impl SpecRepository, service_name: &str, branch_name: &str) -> Result<bool, AppError> {
    Ok(repo.delete_branch(service_name, branch_name).await?)
}

pub async fn delete_branch_all_services(repo: &impl SpecRepository, branch_name: &str) -> Result<u64, AppError> {
    let services = repo.list_services().await?;
    let mut count = 0;
    for svc in services {
        if repo.delete_branch(&svc, branch_name).await? {
            count += 1;
        }
    }
    Ok(count)
}

pub async fn delete_client(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    Ok(repo.delete_client(name).await?)
}

pub async fn list_services(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_services().await?)
}

pub async fn list_services_detailed(repo: &impl SpecRepository) -> Result<Vec<ServiceSummary>, AppError> {
    Ok(repo.list_services_detailed().await?)
}

pub async fn set_fallback_branch(repo: &impl SpecRepository, service_name: &str, branch: Option<&str>) -> Result<(), AppError> {
    repo.set_fallback_branch(service_name, branch).await?;
    Ok(())
}

pub async fn get_fallback_branch(repo: &impl SpecRepository, service_name: &str) -> Result<Option<String>, AppError> {
    Ok(repo.get_fallback_branch(service_name).await?)
}

pub async fn list_branches(repo: &impl SpecRepository, service_name: &str) -> Result<Vec<String>, AppError> {
    Ok(repo.list_branches(service_name).await?)
}

pub async fn list_all_branches(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_all_branches().await?)
}

pub async fn list_clients(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_clients().await?)
}

pub async fn list_client_branches(repo: &impl SpecRepository, client_name: &str) -> Result<Vec<String>, AppError> {
    Ok(repo.list_client_branches(client_name).await?)
}

pub async fn list_client_endpoints(repo: &impl SpecRepository, client_name: &str, branch: &str) -> Result<Vec<ClientEndpointInfo>, AppError> {
    Ok(repo.list_client_endpoints(client_name, branch).await?)
}

pub async fn get_branch_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo.get_setting("branch_max_age_days").await?.unwrap_or("0".to_string());
    Ok(val.parse().unwrap_or(0))
}

pub async fn set_branch_max_age_days(repo: &impl SpecRepository, days: u64) -> Result<(), AppError> {
    repo.set_setting("branch_max_age_days", &days.to_string()).await?;
    Ok(())
}

pub async fn cleanup_stale_branches(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let days = get_branch_max_age_days(repo).await?;
    if days == 0 { return Ok(0); }
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);
    Ok(repo.delete_stale_branches(&cutoff.to_rfc3339()).await?)
}

pub async fn get_dependency_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo.get_setting("dependency_max_age_days").await?.unwrap_or("0".to_string());
    Ok(val.parse().unwrap_or(0))
}

pub async fn set_dependency_max_age_days(repo: &impl SpecRepository, days: u64) -> Result<(), AppError> {
    repo.set_setting("dependency_max_age_days", &days.to_string()).await?;
    Ok(())
}

pub async fn cleanup_stale_dependencies(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let days = get_dependency_max_age_days(repo).await?;
    if days == 0 { return Ok(0); }
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);
    Ok(repo.delete_stale_dependencies(&cutoff.to_rfc3339()).await?)
}
