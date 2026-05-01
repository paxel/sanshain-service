use sanshain_service::application::admin_service as admin;
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::domain::ports::SpecRepository;

// 1. branch_max_age default is 30
#[tokio::test]
async fn branch_max_age_default_30() {
    let repo = MockRepo::new();
    let d = admin::get_branch_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 30);
}

// 2. set/get branch_max_age
#[tokio::test]
async fn branch_max_age_set_get() {
    let repo = MockRepo::new();
    admin::set_branch_max_age_days(&repo, 7).await.unwrap();
    let d = admin::get_branch_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 7);
}

// 3. cleanup_stale_branches returns 0 when days==0 (guard path)
#[tokio::test]
async fn cleanup_stale_branches_zero_days() {
    let repo = MockRepo::new();
    admin::set_branch_max_age_days(&repo, 0).await.unwrap();
    let deleted = admin::cleanup_stale_branches(&repo).await.unwrap();
    assert_eq!(deleted, 0);
}

// 4. dependency_max_age default is 30
#[tokio::test]
async fn dependency_max_age_default_30() {
    let repo = MockRepo::new();
    let d = admin::get_dependency_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 30);
}

// 5. set/get dependency_max_age
#[tokio::test]
async fn dependency_max_age_set_get() {
    let repo = MockRepo::new();
    admin::set_dependency_max_age_days(&repo, 90).await.unwrap();
    let d = admin::get_dependency_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 90);
}

// 6. cleanup_stale_dependencies returns 0 when days==0
#[tokio::test]
async fn cleanup_stale_dependencies_zero_days() {
    let repo = MockRepo::new();
    admin::set_dependency_max_age_days(&repo, 0).await.unwrap();
    let deleted = admin::cleanup_stale_dependencies(&repo).await.unwrap();
    assert_eq!(deleted, 0);
}

// 7. delete non-existing entities: service/client false; branch returns true in MockRepo
#[tokio::test]
async fn delete_nonexisting_returns_false_or_zero() {
    let repo = MockRepo::new();
    assert!(!admin::delete_service(&repo, "nope").await.unwrap());
    assert!(!admin::delete_client(&repo, "nope").await.unwrap());
    assert!(admin::delete_branch(&repo, "svc", "feature").await.unwrap());
}

// 8. list_client_branches empty
#[tokio::test]
async fn list_client_branches_empty() {
    let repo = MockRepo::new();
    let v = admin::list_client_branches(&repo, "c").await.unwrap();
    assert!(v.is_empty());
}

// 9. list_client_endpoints empty
#[tokio::test]
async fn list_client_endpoints_empty() {
    let repo = MockRepo::new();
    let v = admin::list_client_endpoints(&repo, "c", "main").await.unwrap();
    assert!(v.is_empty());
}

// 10. fallback branch set/get roundtrip none -> some -> none
#[tokio::test]
async fn fallback_branch_roundtrip() {
    let repo = MockRepo::new();
    repo.ensure_service("svc").await.unwrap();
    assert!(admin::get_fallback_branch(&repo, "svc").await.unwrap().is_none());
    admin::set_fallback_branch(&repo, "svc", Some("dev")).await.unwrap();
    assert_eq!(admin::get_fallback_branch(&repo, "svc").await.unwrap(), Some("dev".into()));
    admin::set_fallback_branch(&repo, "svc", None).await.unwrap();
    assert!(admin::get_fallback_branch(&repo, "svc").await.unwrap().is_none());
}

// 11. delete_branch_all_services over empty set returns 0
#[tokio::test]
async fn delete_branch_all_services_empty() {
    let repo = MockRepo::new();
    let n = admin::delete_branch_all_services(&repo, "old").await.unwrap();
    assert_eq!(n, 0);
}

// 12. protected branches list contains defaults
#[tokio::test]
async fn protected_branches_have_defaults() {
    let repo = MockRepo::new();
    let list = admin::list_protected_branches(&repo).await.unwrap();
    assert!(list.contains(&"main".to_string()));
    assert!(list.contains(&"master".to_string()));
}
