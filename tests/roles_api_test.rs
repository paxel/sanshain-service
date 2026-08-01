//! End-to-end coverage for the role and group administration API.
//!
//! Runs through the real router so the CSRF layer, the auth middleware and the
//! route wiring are all exercised — a test that called the application layer
//! directly would pass even if the routes were never registered.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::GroupSource;
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::{AppState, create_app};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::RwLock;
use tower::ServiceExt;

const TEST_CSRF_TOKEN: &str = "roles-test-csrf-token";

static TEST_PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
    OnceLock::new();

// `cfg(test)` is always true in this crate; the attribute marks these helpers as
// test code so clippy's `allow-*-in-tests` exemptions apply to them.
#[cfg(test)]
fn prometheus_handle() -> metrics_exporter_prometheus::PrometheusHandle {
    TEST_PROMETHEUS_HANDLE
        .get_or_init(|| {
            let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
            handle
        })
        .clone()
}

#[cfg(test)]
fn test_state(repo: SqliteSpecRepository) -> AppState {
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    let mut system = sysinfo::System::new_all();
    system.refresh_all();

    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 256),
        db_url: "sqlite::memory:".to_string(),
        dev_user: None,
        csrf_tokens: Arc::new(RwLock::new(tokens)),
        instance_id: "test".to_string(),
        spec_updated_tx,
        error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        warn_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        info_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        business_logic_debug: Arc::new(AtomicBool::new(false)),
        admin_user_debug: Arc::new(AtomicBool::new(false)),
        requests_total: Arc::new(AtomicU64::new(0)),
        failures_total: Arc::new(AtomicU64::new(0)),
        process_start_time: Utc::now(),
        prometheus_handle: prometheus_handle(),
        system: Arc::new(std::sync::Mutex::new(system)),
        max_body_bytes: sanshain_service::DEFAULT_MAX_BODY_BYTES,
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
struct Fixture {
    app: axum::Router,
    repo: SqliteSpecRepository,
    admin_token: String,
    plain_user_id: i64,
}

#[cfg(test)]
async fn fixture() -> Fixture {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory database");
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations");

    let hash = services::hash_password("admin-pass").expect("hash");
    let admin = repo
        .create_user("admin", &hash, true)
        .await
        .expect("admin user");
    repo.grant_user_role(admin.id, "admin")
        .await
        .expect("admin grant");
    let admin_session = repo
        .create_session(admin.id, "2099-12-31T23:59:59")
        .await
        .expect("admin session");

    let hash = services::hash_password("user-pass").expect("hash");
    let plain = repo
        .create_user("plain", &hash, true)
        .await
        .expect("plain user");

    let app = create_app(test_state(repo.clone()));
    Fixture {
        app,
        repo,
        admin_token: admin_session.token,
        plain_user_id: plain.id,
    }
}

#[cfg(test)]
async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    csrf: bool,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }
    if csrf {
        builder = builder.header("X-CSRF-Token", TEST_CSRF_TOKEN);
    }
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(Body::from(json.to_string()))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    };

    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

#[tokio::test]
async fn granting_a_role_makes_it_readable_back() {
    let f = fixture().await;

    let (status, _) = call(
        &f.app,
        "POST",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "role": "user_manager" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = call(
        &f.app,
        "GET",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        Some(&f.admin_token),
        false,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["roles"], serde_json::json!(["user_manager"]));
}

#[tokio::test]
async fn revoking_a_role_removes_it() {
    let f = fixture().await;
    let uri = format!("/admin/users/{}/roles", f.plain_user_id);

    call(
        &f.app,
        "POST",
        &uri,
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "role": "viewer" })),
    )
    .await;

    let (status, _) = call(
        &f.app,
        "DELETE",
        &format!("{}/viewer", uri),
        Some(&f.admin_token),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = call(&f.app, "GET", &uri, Some(&f.admin_token), false, None).await;
    assert_eq!(body["roles"], serde_json::json!([]));
}

#[tokio::test]
async fn revoking_a_role_the_user_does_not_hold_is_not_found() {
    let f = fixture().await;
    let (status, _) = call(
        &f.app,
        "DELETE",
        &format!("/admin/users/{}/roles/viewer", f.plain_user_id),
        Some(&f.admin_token),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn maintainer_cannot_be_granted_instance_wide() {
    let f = fixture().await;
    let (status, body) = call(
        &f.app,
        "POST",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "role": "maintainer" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("scoped to specific Producers"),
        "unexpected message: {}",
        body
    );
}

#[tokio::test]
async fn an_unknown_role_is_refused() {
    let f = fixture().await;
    let (status, _) = call(
        &f.app,
        "POST",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "role": "wizard" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The CSRF layer is what stops a browser being walked into granting a role, so
/// it is asserted directly rather than assumed from the router wiring.
///
/// The check is on ambient credentials only: a request carrying a `Bearer`
/// token is exempt by design, because a cross-site request cannot set that
/// header. So the case that must be refused is the one a browser could be made
/// to send — no bearer token, no CSRF token.
#[tokio::test]
async fn a_state_changing_request_without_a_csrf_token_is_refused() {
    let f = fixture().await;
    let (status, _) = call(
        &f.app,
        "POST",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        None,
        false,
        Some(serde_json::json!({ "role": "viewer" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let roles = f
        .repo
        .list_user_roles(f.plain_user_id)
        .await
        .expect("roles readable");
    assert!(roles.is_empty(), "the refused grant must not have landed");
}

/// A stale CSRF token must not pass, or the check would be satisfied by any
/// string at all.
#[tokio::test]
async fn a_state_changing_request_with_an_unknown_csrf_token_is_refused() {
    let f = fixture().await;
    let request = Request::builder()
        .method("POST")
        .uri(format!("/admin/users/{}/roles", f.plain_user_id))
        .header("Content-Type", "application/json")
        .header("X-CSRF-Token", "not-a-real-token")
        .body(Body::from(
            serde_json::json!({ "role": "viewer" }).to_string(),
        ))
        .expect("request");
    let response = f.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn role_administration_requires_authentication() {
    let f = fixture().await;
    let (status, _) = call(
        &f.app,
        "GET",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_group_carries_roles_to_its_members() {
    let f = fixture().await;

    let (status, group) = call(
        &f.app,
        "POST",
        "/admin/groups",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "name": "platform" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let group_id = group["id"].as_i64().expect("group id");
    assert_eq!(group["source"], "native");

    let (status, _) = call(
        &f.app,
        "PUT",
        &format!("/admin/groups/{}", group_id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "roles": ["viewer"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(
        &f.app,
        "POST",
        &format!("/admin/groups/{}/members", group_id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "user_id": f.plain_user_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // The role is held through the group, not granted directly.
    let direct = f
        .repo
        .list_user_roles(f.plain_user_id)
        .await
        .expect("direct roles");
    assert!(direct.is_empty());
    let effective = f
        .repo
        .effective_stored_roles(f.plain_user_id)
        .await
        .expect("effective roles");
    assert_eq!(effective, vec!["viewer".to_string()]);
}

#[tokio::test]
async fn removing_a_member_removes_the_role_they_held_through_the_group() {
    let f = fixture().await;
    let group = f
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");
    f.repo
        .set_group_roles(group.id, &["viewer".to_string()])
        .await
        .expect("roles");
    f.repo
        .add_group_member(group.id, f.plain_user_id)
        .await
        .expect("member");

    let (status, _) = call(
        &f.app,
        "DELETE",
        &format!("/admin/groups/{}/members/{}", group.id, f.plain_user_id),
        Some(&f.admin_token),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let effective = f
        .repo
        .effective_stored_roles(f.plain_user_id)
        .await
        .expect("effective roles");
    assert!(effective.is_empty());
}

#[tokio::test]
async fn a_directory_group_refuses_membership_edits_but_accepts_roles() {
    let f = fixture().await;
    let group = f
        .repo
        .create_group("ad-admins", GroupSource::Ldap)
        .await
        .expect("directory group");

    let (status, _) = call(
        &f.app,
        "POST",
        &format!("/admin/groups/{}/members", group.id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "user_id": f.plain_user_id })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &f.app,
        "PUT",
        &format!("/admin/groups/{}", group.id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "roles": ["admin"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        f.repo
            .list_group_roles(group.id)
            .await
            .expect("group roles"),
        vec!["admin".to_string()]
    );
}

#[tokio::test]
async fn a_directory_group_cannot_be_renamed() {
    let f = fixture().await;
    let group = f
        .repo
        .create_group("ad-admins", GroupSource::Ldap)
        .await
        .expect("directory group");

    let (status, _) = call(
        &f.app,
        "PUT",
        &format!("/admin/groups/{}", group.id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "name": "renamed" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn groups_are_listed_with_their_origin_roles_and_members() {
    let f = fixture().await;
    let native = f
        .repo
        .create_group("developers", GroupSource::Native)
        .await
        .expect("native group");
    f.repo
        .create_group("developers", GroupSource::Ldap)
        .await
        .expect("directory group of the same name");
    f.repo
        .set_group_roles(native.id, &["viewer".to_string()])
        .await
        .expect("roles");
    f.repo
        .add_group_member(native.id, f.plain_user_id)
        .await
        .expect("member");

    let (status, body) = call(
        &f.app,
        "GET",
        "/admin/groups",
        Some(&f.admin_token),
        false,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let groups = body["groups"].as_array().expect("groups array");
    assert_eq!(groups.len(), 2, "same name, different origin, both listed");

    let listed_native = groups
        .iter()
        .find(|g| g["source"] == "native")
        .expect("native group listed");
    assert_eq!(listed_native["name"], "developers");
    assert_eq!(listed_native["roles"], serde_json::json!(["viewer"]));
    assert_eq!(
        listed_native["member_ids"],
        serde_json::json!([f.plain_user_id])
    );

    let listed_directory = groups
        .iter()
        .find(|g| g["source"] == "ldap")
        .expect("directory group listed");
    assert_eq!(
        listed_directory["member_ids"],
        serde_json::json!([]),
        "a directory group stores no membership"
    );
}

#[tokio::test]
async fn deleting_a_group_takes_its_grants_with_it() {
    let f = fixture().await;
    let group = f
        .repo
        .create_group("temp", GroupSource::Native)
        .await
        .expect("group");
    f.repo
        .set_group_roles(group.id, &["viewer".to_string()])
        .await
        .expect("roles");
    f.repo
        .add_group_member(group.id, f.plain_user_id)
        .await
        .expect("member");

    let (status, _) = call(
        &f.app,
        "DELETE",
        &format!("/admin/groups/{}", group.id),
        Some(&f.admin_token),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let effective = f
        .repo
        .effective_stored_roles(f.plain_user_id)
        .await
        .expect("effective roles");
    assert!(effective.is_empty());
}

#[tokio::test]
async fn an_empty_group_update_is_refused() {
    let f = fixture().await;
    let group = f
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");

    let (status, _) = call(
        &f.app,
        "PUT",
        &format!("/admin/groups/{}", group.id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_role_catalogue_reports_what_each_role_confers() {
    let f = fixture().await;
    let (status, body) = call(
        &f.app,
        "GET",
        "/admin/roles",
        Some(&f.admin_token),
        false,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let roles = body["roles"].as_array().expect("roles array");
    assert_eq!(roles.len(), 4);

    let admin = roles
        .iter()
        .find(|r| r["role"] == "admin")
        .expect("admin listed");
    let permissions = body["permissions"].as_array().expect("permission list");
    assert_eq!(
        admin["permissions"]
            .as_array()
            .expect("admin permissions")
            .len(),
        permissions.len(),
        "admin holds every permission"
    );
    assert_eq!(admin["globally_grantable"], true);

    let maintainer = roles
        .iter()
        .find(|r| r["role"] == "maintainer")
        .expect("maintainer listed");
    assert_eq!(maintainer["globally_grantable"], false);
}

#[tokio::test]
async fn role_administration_is_recorded_in_the_audit_log() {
    let f = fixture().await;
    call(
        &f.app,
        "POST",
        &format!("/admin/users/{}/roles", f.plain_user_id),
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "role": "viewer" })),
    )
    .await;

    let logs = f.repo.get_recent_audit_logs(50).await.expect("audit logs");
    assert!(
        logs.iter()
            .any(|entry| entry.action == "GRANT_ROLE" && entry.username == "admin"),
        "the grant should be attributed to the admin who made it"
    );
}

// --- Maintainer scope ---

#[tokio::test]
async fn a_producer_reports_its_user_and_group_maintainers() {
    let f = fixture().await;
    f.repo.ensure_service("orders").await.expect("producer");
    let group = f
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");

    let (status, _) = call(
        &f.app,
        "POST",
        "/admin/producers/orders/maintainers",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "user_id": f.plain_user_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = call(
        &f.app,
        "POST",
        "/admin/producers/orders/maintainers",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "group_id": group.id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = call(
        &f.app,
        "GET",
        "/admin/producers/orders/maintainers",
        Some(&f.admin_token),
        false,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user_ids"], serde_json::json!([f.plain_user_id]));
    assert_eq!(body["group_ids"], serde_json::json!([group.id]));
}

/// A row assigns either a user or a group. Accepting both in one request would
/// quietly make two assignments; accepting neither would make none.
#[tokio::test]
async fn assigning_needs_exactly_one_of_user_or_group() {
    let f = fixture().await;
    f.repo.ensure_service("orders").await.expect("producer");
    let group = f
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");

    for payload in [
        serde_json::json!({}),
        serde_json::json!({ "user_id": f.plain_user_id, "group_id": group.id }),
    ] {
        let (status, _) = call(
            &f.app,
            "POST",
            "/admin/producers/orders/maintainers",
            Some(&f.admin_token),
            true,
            Some(payload),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn unassigning_a_maintainer_who_is_not_one_is_not_found() {
    let f = fixture().await;
    f.repo.ensure_service("orders").await.expect("producer");
    let (status, _) = call(
        &f.app,
        "DELETE",
        &format!(
            "/admin/producers/orders/maintainers/users/{}",
            f.plain_user_id
        ),
        Some(&f.admin_token),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assigning_a_maintainer_to_an_unknown_producer_is_not_found() {
    let f = fixture().await;
    let (status, _) = call(
        &f.app,
        "POST",
        "/admin/producers/nope/maintainers",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "user_id": f.plain_user_id })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_users_maintained_producers_cover_direct_and_group_assignments() {
    let f = fixture().await;
    f.repo.ensure_service("orders").await.expect("producer");
    f.repo.ensure_service("shipping").await.expect("producer");
    f.repo.ensure_service("billing").await.expect("producer");
    let group = f
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");
    f.repo
        .add_group_member(group.id, f.plain_user_id)
        .await
        .expect("member");

    call(
        &f.app,
        "POST",
        "/admin/producers/orders/maintainers",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "user_id": f.plain_user_id })),
    )
    .await;
    call(
        &f.app,
        "POST",
        "/admin/producers/shipping/maintainers",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "group_id": group.id })),
    )
    .await;

    let (status, body) = call(
        &f.app,
        "GET",
        &format!("/admin/users/{}/maintains", f.plain_user_id),
        Some(&f.admin_token),
        false,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["producers"],
        serde_json::json!(["orders", "shipping"]),
        "billing is maintained by nobody"
    );
}

#[tokio::test]
async fn maintainer_assignment_is_recorded_in_the_audit_log() {
    let f = fixture().await;
    f.repo.ensure_service("orders").await.expect("producer");
    call(
        &f.app,
        "POST",
        "/admin/producers/orders/maintainers",
        Some(&f.admin_token),
        true,
        Some(serde_json::json!({ "user_id": f.plain_user_id })),
    )
    .await;

    let logs = f.repo.get_recent_audit_logs(50).await.expect("audit logs");
    assert!(
        logs.iter()
            .any(|e| e.action == "ASSIGN_MAINTAINER" && e.username == "admin"),
        "the assignment should be attributed to the admin who made it"
    );
}
