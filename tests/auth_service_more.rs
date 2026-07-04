use sanshain_service::application::auth_service as auth;
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;

// 1. dev_mode default false
#[tokio::test]
async fn dev_mode_default_false() {
    let repo = MockRepo::new();
    assert!(!auth::get_dev_mode(&repo).await.unwrap());
}

// 2. set dev mode true — persists the request, but stays *inactive* without the
// ALLOW_INSECURE_DEV_MODE safety gate (which is not set in the test process).
#[tokio::test]
async fn dev_mode_set_true_is_refused_without_gate() {
    let repo = MockRepo::new();
    auth::set_dev_mode(&repo, true).await.unwrap();
    // Request is recorded...
    assert!(auth::is_dev_mode_requested(&repo).await.unwrap());
    // ...but dev mode fails closed because the safety gate is not open.
    assert!(!auth::dev_mode_gate_open());
    assert!(!auth::get_dev_mode(&repo).await.unwrap());
}

// 3. get_auth_mode default is Disabled
#[tokio::test]
async fn auth_mode_default_disabled() {
    let repo = MockRepo::new();
    let mode = auth::get_auth_mode(&repo).await.unwrap();
    assert!(matches!(mode, AuthMode::Disabled));
}

// 4. set_auth_mode to Local and verify
#[tokio::test]
async fn set_auth_mode_local() {
    let repo = MockRepo::new();
    auth::set_auth_mode(&repo, &AuthMode::Local).await.unwrap();
    let mode = auth::get_auth_mode(&repo).await.unwrap();
    assert!(matches!(mode, AuthMode::Local));
}

// 5. auto-approve default false, set true
#[tokio::test]
async fn auto_approve_toggle() {
    let repo = MockRepo::new();
    assert!(!auth::get_auto_approve_users(&repo).await.unwrap());
    auth::set_auto_approve_users(&repo, true).await.unwrap();
    assert!(auth::get_auto_approve_users(&repo).await.unwrap());
}

// 6. auth mode roundtrip ldap
#[tokio::test]
async fn auth_mode_roundtrip_ldap() {
    let repo = MockRepo::new();
    auth::set_auth_mode(&repo, &AuthMode::Ldap).await.unwrap();
    let mode = auth::get_auth_mode(&repo).await.unwrap();
    assert!(matches!(mode, AuthMode::Ldap));
}

// 7. login unknown user => Unauthorized
#[tokio::test]
async fn login_unknown_user_unauthorized() {
    let repo = MockRepo::new();
    let res = auth::login(&repo, "nouser", "pw").await;
    assert!(matches!(res, Err(AppError::Unauthorized)));
}

// 8. change_password unauthorized on wrong old password
#[tokio::test]
async fn change_password_wrong_old_password() {
    let repo = MockRepo::new();
    let hash = auth::hash_password("old").unwrap();
    let user = User {
        id: 1,
        username: "u".into(),
        password_hash: hash,
        is_admin: false,
        approved: true,
    };
    let res = auth::change_password(&repo, &user, None, "bad", "new").await;
    assert!(matches!(res, Err(AppError::Unauthorized)));
}

// 9. verify_password true/false paths
#[test]
fn verify_password_true_false() {
    let hash = auth::hash_password("secret").unwrap();
    assert!(auth::verify_password("secret", &hash).unwrap());
    assert!(!auth::verify_password("wrong", &hash).unwrap());
}

// 10. generate_random_password length 16
#[test]
fn random_password_length() {
    let p = auth::generate_random_password();
    assert_eq!(p.len(), 16);
}

// 11. get/set ldap config roundtrip with tls true
#[tokio::test]
async fn ldap_config_roundtrip_tls() {
    let repo = MockRepo::new();
    let cfg = LdapConfig {
        server_url: "ldaps://host".into(),
        bind_dn: "cn=a".into(),
        bind_password: Some("pw".into()),
        base_dn: "dc=x".into(),
        user_filter: "(uid={username})".into(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: true,
    };
    auth::set_ldap_config(&repo, &cfg).await.unwrap();
    let got = auth::get_ldap_config(&repo).await.unwrap().unwrap();
    assert_eq!(got.server_url, "ldaps://host");
    assert!(got.use_tls);
}

// 12. register_user conflict when same username exists
#[tokio::test]
async fn register_user_conflict() {
    let repo = MockRepo::new();
    // enable and no auto-approve just for path
    auth::set_auth_mode(&repo, &AuthMode::Local).await.unwrap();
    let hash = auth::hash_password("pw").unwrap();
    repo.create_user("bob", &hash, false, true).await.unwrap();
    let res = auth::register_user(&repo, "bob", "pw").await;
    match res {
        Err(AppError::Conflict(_)) => {}
        _ => panic!("expected conflict"),
    }
}
