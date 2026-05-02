use sanshain_service::application::auth_service;
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;

// 1. get_auth_mode default is Dev
#[tokio::test]
async fn auth_mode_default_dev() {
    let repo = MockRepo::new();
    let mode = auth_service::get_auth_mode(&repo).await.unwrap();
    assert!(matches!(mode, AuthMode::Dev));
}

// 2. set_auth_mode to Local and verify
#[tokio::test]
async fn set_auth_mode_local() {
    let repo = MockRepo::new();
    auth_service::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    let mode = auth_service::get_auth_mode(&repo).await.unwrap();
    assert!(matches!(mode, AuthMode::Local));
}

// 3. set/get ldap config roundtrip
#[tokio::test]
async fn ldap_config_roundtrip() {
    let repo = MockRepo::new();
    let cfg = LdapConfig {
        server_url: "ldap://localhost:389".to_string(),
        bind_dn: "cn=admin,dc=example,dc=org".to_string(),
        bind_password: Some("pw".to_string()),
        base_dn: "dc=example,dc=org".to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: false,
    };
    auth_service::set_ldap_config(&repo, &cfg).await.unwrap();
    let got = auth_service::get_ldap_config(&repo).await.unwrap().unwrap();
    assert_eq!(got.server_url, cfg.server_url);
    assert_eq!(got.bind_dn, cfg.bind_dn);
    assert_eq!(got.base_dn, cfg.base_dn);
    assert_eq!(got.use_tls, cfg.use_tls);
}

// 4. LdapConfig::validate success
#[test]
fn ldap_validate_ok() {
    let cfg = LdapConfig {
        server_url: "ldaps://ldap.example.org".to_string(),
        bind_dn: "cn=admin,dc=example,dc=org".to_string(),
        bind_password: None,
        base_dn: "dc=example,dc=org".to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: true,
    };
    assert!(cfg.validate().is_ok());
}

// 5-8. LdapConfig::validate errors on bad inputs
#[test]
fn ldap_validate_bad_url() {
    let cfg = LdapConfig {
        server_url: "http://bad".to_string(),
        bind_dn: "x".to_string(),
        bind_password: None,
        base_dn: "y".to_string(),
        user_filter: String::new(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: false,
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn ldap_validate_empty_bind_dn() {
    let cfg = LdapConfig {
        server_url: "ldap://x".to_string(),
        bind_dn: String::new(),
        bind_password: None,
        base_dn: "y".to_string(),
        user_filter: String::new(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: false,
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn ldap_validate_empty_base_dn() {
    let cfg = LdapConfig {
        server_url: "ldap://x".to_string(),
        bind_dn: "cn=admin".to_string(),
        bind_password: None,
        base_dn: String::new(),
        user_filter: String::new(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: false,
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn ldap_validate_bad_host() {
    let cfg = LdapConfig {
        server_url: "ldap:///".to_string(),
        bind_dn: "cn=a".to_string(),
        bind_password: None,
        base_dn: "dc=x".to_string(),
        user_filter: String::new(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: false,
    };
    assert!(cfg.validate().is_err());
}

// 9. approve_user: returns false when id not found
#[tokio::test]
async fn approve_user_not_found() {
    let repo = MockRepo::new();
    let ok = auth_service::approve_user(&repo, 123).await.unwrap();
    assert!(!ok);
}

// 10. admin_delete_user: returns false when id not found
#[tokio::test]
async fn admin_delete_user_not_found() {
    let repo = MockRepo::new();
    let ok = auth_service::admin_delete_user(&repo, 123).await.unwrap();
    assert!(!ok);
}

// 11. register_user forbidden when local users disabled
#[tokio::test]
async fn register_user_forbidden_when_disabled() {
    let repo = MockRepo::new();
    auth_service::set_local_users_enabled(&repo, false)
        .await
        .unwrap();
    let res = auth_service::register_user(&repo, "u", "p").await;
    assert!(matches!(res, Err(AppError::Forbidden)));
}

// 12. register_user ok when enabled and auto-approve true
#[tokio::test]
async fn register_user_ok_when_enabled_autoapprove() {
    let repo = MockRepo::new();
    auth_service::set_local_users_enabled(&repo, true)
        .await
        .unwrap();
    auth_service::set_auto_approve_users(&repo, true)
        .await
        .unwrap();
    auth_service::register_user(&repo, "bob", "pw")
        .await
        .unwrap();
    let users = auth_service::list_users(&repo).await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].username, "bob");
    assert!(users[0].approved);
}
