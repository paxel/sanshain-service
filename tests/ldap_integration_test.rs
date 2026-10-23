//! LDAP authentication against a real OpenLDAP server (testcontainers).
//!
//! The provider was previously covered only by config-validation unit tests —
//! the actual bind/search/group flow never ran. This exercises it end to end
//! over plain `ldap://`: the happy path with group resolution, the credential
//! and lookup failure modes, LDAP filter-injection escaping, a bad
//! service-account bind, and the connect timeout. The `ldaps://`/StartTLS
//! matrix lives in `ldap_tls_integration_test.rs` (a separate binary, because
//! the client trust store is a process-wide `OnceLock`).

use ldap3::{Ldap, LdapConnAsync, Mod};
use sanshain_service::domain::models::LdapConfig;
use sanshain_service::domain::ports::DirectoryGroupSearch;
use sanshain_service::domain::ports::{AuthProvider, AuthProviderError, DirectoryGroups};
use sanshain_service::infrastructure::ldap_provider::LdapAuthProvider;
use std::collections::HashSet;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const LDAP_PORT: u16 = 389;
const LDAP_BASE_DN: &str = "dc=example,dc=org";
const LDAP_ADMIN_DN: &str = "cn=admin,dc=example,dc=org";
const LDAP_ADMIN_PW: &str = "adminpw";
const ADMINS_DN: &str = "cn=admins,ou=groups,dc=example,dc=org";
const ALICE_DN: &str = "uid=alice,ou=people,dc=example,dc=org";

/// Start a throwaway OpenLDAP container (osixia/openldap), retrying transient
/// pull/start failures (same bollard concurrent-pull flake the Postgres helper
/// guards against). osixia enables the `memberof` overlay by default, so a
/// user's `memberOf` is populated from the `groupOfNames` it belongs to.
#[cfg(test)]
async fn start_openldap() -> ContainerAsync<GenericImage> {
    let mut last_err = String::new();
    for attempt in 1..=5 {
        let image = GenericImage::new("osixia/openldap", "1.5.0")
            .with_wait_for(WaitFor::message_on_stderr("slapd starting"))
            .with_exposed_port(LDAP_PORT.tcp())
            .with_env_var("LDAP_ORGANISATION", "Example Inc.")
            .with_env_var("LDAP_DOMAIN", "example.org")
            .with_env_var("LDAP_ADMIN_PASSWORD", LDAP_ADMIN_PW)
            .with_env_var("LDAP_CONFIG_PASSWORD", "configpw");
        match image.start().await {
            Ok(container) => return container,
            Err(err) => {
                last_err = err.to_string();
                eprintln!("openldap container start attempt {attempt}/5 failed: {last_err}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    panic!("openldap container failed to start after 5 attempts: {last_err}");
}

fn attr<'a>(values: &[&'a str]) -> HashSet<&'a str> {
    values.iter().copied().collect()
}

/// Bind as the directory admin, retrying while osixia finishes its first-boot
/// bootstrap (the "slapd starting" log precedes readiness for writes).
#[cfg(test)]
async fn admin_bind(url: &str) -> Ldap {
    let mut last = String::new();
    for _ in 0..20 {
        match LdapConnAsync::new(url).await {
            Ok((conn, mut ldap)) => {
                ldap3::drive!(conn);
                match ldap.simple_bind(LDAP_ADMIN_DN, LDAP_ADMIN_PW).await {
                    Ok(r) => match r.success() {
                        Ok(_) => return ldap,
                        Err(e) => last = e.to_string(),
                    },
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("admin bind never succeeded: {last}");
}

/// Enable the memberof overlay via `cn=config` so a `groupOfNames` membership
/// populates the member's `memberOf` (which is what the provider reads). osixia
/// does not enable it by default. Must run *before* the groups are seeded — the
/// overlay populates `memberOf` on member additions made while it is active.
#[cfg(test)]
async fn enable_memberof(url: &str) {
    let (conn, mut ldap) = LdapConnAsync::new(url).await.expect("connect for config");
    ldap3::drive!(conn);
    ldap.simple_bind("cn=admin,cn=config", "configpw")
        .await
        .expect("config bind request")
        .success()
        .expect("config bind");
    // Ensure the memberof module is loaded (no-op/soft-fail if already present).
    let _ = ldap
        .modify(
            "cn=module{0},cn=config",
            vec![Mod::Add("olcModuleLoad", attr(&["memberof"]))],
        )
        .await;
    ldap.add(
        "olcOverlay=memberof,olcDatabase={1}mdb,cn=config",
        vec![
            ("objectClass", attr(&["olcOverlayConfig", "olcMemberOf"])),
            ("olcOverlay", attr(&["memberof"])),
        ],
    )
    .await
    .unwrap_or_else(|e| panic!("add memberof overlay: {e}"))
    .success()
    .unwrap_or_else(|e| panic!("memberof overlay rejected: {e}"));
    let _ = ldap.unbind().await;
}

/// Seed people/groups: user `alice` and group `admins` with `alice` as a
/// member (the memberof overlay then populates `alice`'s `memberOf`).
#[cfg(test)]
async fn seed(url: &str) {
    let mut ldap = admin_bind(url).await;
    for (dn, attrs) in [
        (
            "ou=people,dc=example,dc=org",
            vec![
                ("objectClass", attr(&["organizationalUnit"])),
                ("ou", attr(&["people"])),
            ],
        ),
        (
            "ou=groups,dc=example,dc=org",
            vec![
                ("objectClass", attr(&["organizationalUnit"])),
                ("ou", attr(&["groups"])),
            ],
        ),
        (
            ALICE_DN,
            vec![
                ("objectClass", attr(&["inetOrgPerson"])),
                ("cn", attr(&["Alice"])),
                ("sn", attr(&["Anderson"])),
                ("uid", attr(&["alice"])),
                ("displayName", attr(&["Alice Anderson"])),
                ("mail", attr(&["Alice.Anderson@Example.org"])),
                ("userPassword", attr(&["alicepw"])),
            ],
        ),
        (
            ADMINS_DN,
            vec![
                ("objectClass", attr(&["groupOfNames"])),
                ("cn", attr(&["admins"])),
                ("member", attr(&[ALICE_DN])),
            ],
        ),
    ] {
        ldap.add(dn, attrs)
            .await
            .unwrap_or_else(|e| panic!("add {dn}: {e}"))
            .success()
            .unwrap_or_else(|e| panic!("add {dn} rejected: {e}"));
    }
    let _ = ldap.unbind().await;
}

fn config(url: &str) -> LdapConfig {
    LdapConfig {
        server_url: url.to_string(),
        bind_dn: LDAP_ADMIN_DN.to_string(),
        bind_password: Some(LDAP_ADMIN_PW.to_string()),
        base_dn: LDAP_BASE_DN.to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: ADMINS_DN.to_string(),
        use_tls: false,
        username_attribute: String::new(),
    }
}

#[tokio::test]
async fn ldap_plain_bind_search_groups_and_failure_modes() {
    let container = start_openldap().await;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(LDAP_PORT).await.unwrap();
    let url = format!("ldap://{host}:{port}");
    // Gate on slapd readiness, enable the memberof overlay, then seed so the
    // group membership populates alice's memberOf.
    let mut ready = admin_bind(&url).await;
    let _ = ready.unbind().await;
    enable_memberof(&url).await;
    seed(&url).await;

    let provider = LdapAuthProvider::new(config(&url));

    // Happy path: valid credentials authenticate.
    let user = provider
        .authenticate("alice", "alicepw")
        .await
        .expect("valid credentials authenticate");
    assert_eq!(user.username, "alice");
    assert_eq!(user.display_name.as_deref(), Some("Alice Anderson"));
    let id = user.directory_id.clone().expect("entryUUID read");
    assert!(id.starts_with("uuid:"), "{id}");

    // A filter that also accepts the e-mail address: either name signs in as
    // the same person, under the uid, with the same directory ID.
    let mut either = config(&url);
    either.user_filter = "(|(uid={username})(mail={username}))".to_string();
    let by_mail = LdapAuthProvider::new(either.clone())
        .authenticate("Alice.Anderson@Example.org", "alicepw")
        .await
        .expect("the e-mail address signs in");
    assert_eq!(by_mail.username, "alice");
    assert_eq!(by_mail.directory_id.as_deref(), Some(id.as_str()));
    // A configured attribute wins over uid.
    either.username_attribute = "mail".to_string();
    let named_by_mail = LdapAuthProvider::new(either)
        .authenticate("alice", "alicepw")
        .await
        .expect("signs in");
    assert_eq!(named_by_mail.username, "Alice.Anderson@Example.org");
    assert_eq!(user.email.as_deref(), Some("Alice.Anderson@Example.org"));

    // A group found by a part of its name, by its DN; an injected filter
    // finds nothing instead of everything.
    let found = provider
        .find_groups("dmin", 20)
        .await
        .expect("group search");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].dn.eq_ignore_ascii_case(ADMINS_DN));
    assert_eq!(found[0].name, "admins");
    assert!(
        provider
            .find_groups("*)(cn=*", 20)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(provider.find_groups("  ", 20).await.unwrap().is_empty());

    // Group resolution via memberOf → the admins group.
    let groups = provider.groups_for("alice").await.expect("groups resolve");
    assert!(
        groups.iter().any(|g| g.eq_ignore_ascii_case(ADMINS_DN)),
        "memberOf must include the admins group, got: {groups:?}"
    );

    // Wrong password: user found, user-bind fails → InvalidCredentials.
    assert!(matches!(
        provider.authenticate("alice", "wrong").await,
        Err(AuthProviderError::InvalidCredentials)
    ));

    // Unknown user: search empty → InvalidCredentials.
    assert!(matches!(
        provider.authenticate("nobody", "x").await,
        Err(AuthProviderError::InvalidCredentials)
    ));

    // Filter injection: a crafted username must be escaped, not bypass auth.
    assert!(matches!(
        provider.authenticate("*)(uid=*", "x").await,
        Err(AuthProviderError::InvalidCredentials)
    ));

    // Bad service-account bind → ConnectionFailed (before any user lookup).
    let mut bad = config(&url);
    bad.bind_password = Some("not-the-admin-password".to_string());
    assert!(matches!(
        LdapAuthProvider::new(bad)
            .authenticate("alice", "alicepw")
            .await,
        Err(AuthProviderError::ConnectionFailed(_))
    ));
}

/// An unreachable directory fails fast (bounded by CONNECT_TIMEOUT), not after
/// the OS TCP timeout. No container needed.
#[tokio::test]
async fn ldap_unreachable_server_fails_fast() {
    // 127.0.0.1:1 refuses immediately; the point is the error class, and that
    // the call returns rather than hanging.
    let provider = LdapAuthProvider::new(config("ldap://127.0.0.1:1"));
    let started = std::time::Instant::now();
    let result = provider.authenticate("alice", "alicepw").await;
    assert!(
        matches!(result, Err(AuthProviderError::Unreachable(_))),
        "a closed port means the directory was never reached, got: {result:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "must fail fast, took {:?}",
        started.elapsed()
    );
}

// --- The admin page's way to a directory team, end to end over HTTP ---

#[cfg(test)]
fn app_state(
    repo: sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository,
) -> sanshain_service::AppState {
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::{Arc, OnceLock};
    static PROMETHEUS: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(16);
    sanshain_service::AppState {
        repo: sanshain_service::infrastructure::cached_repository::CachedSpecRepository::new(
            sanshain_service::infrastructure::database::DatabaseRepo::Sqlite(repo),
            64,
        ),
        db_url: "sqlite::memory:".into(),
        dev_user: None,
        csrf_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        instance_id: "test".into(),
        spec_updated_tx,
        error_buffer: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        warn_buffer: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        info_buffer: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        debug_buffer: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        business_logic_debug: Arc::new(AtomicBool::new(false)),
        admin_user_debug: Arc::new(AtomicBool::new(false)),
        requests_total: Arc::new(AtomicU64::new(0)),
        failures_total: Arc::new(AtomicU64::new(0)),
        process_start_time: chrono::Utc::now(),
        prometheus_handle: PROMETHEUS
            .get_or_init(|| axum_prometheus::PrometheusMetricLayer::pair().1)
            .clone(),
        system: Arc::new(std::sync::Mutex::new(sysinfo::System::new_all())),
        max_body_bytes: sanshain_service::DEFAULT_MAX_BODY_BYTES,
        snapshot_in_progress: Arc::new(AtomicBool::new(false)),
        mcp: Default::default(),
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            Some("root"),
            None,
        )),
    }
}

#[cfg(test)]
async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(json.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(request).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

/// The team `carla` in the directory is found by name on the admin page, added,
/// made maintainer of `penguin`, and alice, a member, maintains it once she
/// signs in through LDAP — nobody typed a DN, and Sanshain stores no member.
#[tokio::test]
async fn a_directory_team_found_on_the_admin_page_maintains_a_producer() {
    use axum::http::StatusCode;
    use sanshain_service::application::services;
    use sanshain_service::domain::models::AuthMode;
    use sanshain_service::domain::ports::SpecRepository;
    use serde_json::json;

    let container = start_openldap().await;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(LDAP_PORT).await.unwrap();
    let url = format!("ldap://{host}:{port}");
    let mut ready = admin_bind(&url).await;
    let _ = ready.unbind().await;
    enable_memberof(&url).await;
    seed(&url).await;
    let carla = "cn=carla,ou=groups,dc=example,dc=org";
    let mut ldap = admin_bind(&url).await;
    ldap.add(
        carla,
        vec![
            ("objectClass", attr(&["groupOfNames"])),
            ("cn", attr(&["carla"])),
            ("member", attr(&[ALICE_DN])),
        ],
    )
    .await
    .unwrap()
    .success()
    .unwrap();
    let _ = ldap.unbind().await;

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    let mut ldap_config = config(&url);
    // Only the team grants anything here, not the admins group.
    ldap_config.admin_group = String::new();
    services::set_ldap_config(&repo, &ldap_config)
        .await
        .unwrap();
    services::set_auth_mode(&repo, &AuthMode::Ldap)
        .await
        .unwrap();
    let hash = services::hash_password("root-pass").unwrap();
    let root = repo.create_user("root", &hash, true).await.unwrap();
    let root_token = repo
        .create_session(root.id, "2099-12-31T23:59:59")
        .await
        .unwrap()
        .token;
    repo.ensure_service("penguin").await.unwrap();
    let app = sanshain_service::create_app(app_state(repo));

    // Found by a part of its name, not yet known to Sanshain.
    let (status, found) = call(
        &app,
        "GET",
        "/admin/directory-groups?q=carl",
        Some(&root_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let hits = found["groups"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{found}");
    assert_eq!(hits[0]["name"], "carla");
    assert!(hits[0]["group_id"].is_null());
    let dn = hits[0]["dn"].as_str().unwrap().to_string();

    let (status, group) = call(
        &app,
        "POST",
        "/admin/groups/directory",
        Some(&root_token),
        Some(json!({ "dn": dn })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let (status, body) = call(
        &app,
        "POST",
        "/admin/producers/penguin/maintainers",
        Some(&root_token),
        Some(json!({ "group_id": group["id"] })),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let (_, found) = call(
        &app,
        "GET",
        "/admin/directory-groups?q=carl",
        Some(&root_token),
        None,
    )
    .await;
    assert_eq!(found["groups"][0]["group_id"], group["id"], "now known");

    // Alice signs in through the directory and maintains penguin by her team.
    let (status, login) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "username": "alice", "password": "alicepw" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let alice_token = login["token"].as_str().unwrap();
    let (status, me) = call(&app, "GET", "/auth/me", Some(alice_token), None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["sign_in"], "ldap");
    assert_eq!(me["display_name"], "Alice Anderson");
    assert_eq!(me["maintains"], json!(["penguin"]), "{me}");
}
