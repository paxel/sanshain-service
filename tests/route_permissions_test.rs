//! What each kind of caller can actually reach.
//!
//! The build-time lint in `lib.rs` proves every admin route *declares* a
//! requirement; it cannot prove the requirement is enforced, or that the
//! declaration is the right one. These tests drive real requests through the
//! router to check both.

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

const TEST_CSRF_TOKEN: &str = "route-permissions-csrf-token";

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
            Some("breakglass"),
            None,
        )),
    }
}

#[cfg(test)]
struct World {
    app: axum::Router,
    repo: SqliteSpecRepository,
    admin: String,
    plain: String,
    user_manager: String,
    maintainer: String,
    root: String,
}

#[cfg(test)]
async fn world() -> World {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory database");
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations");

    let hash = services::hash_password("pw").expect("hash");

    // `token_for` needs the user to exist first, so each is created then given a
    // session.
    let mut tokens = Vec::new();
    for (name, admin) in [
        ("admin", true),
        ("plain", false),
        ("user_manager", false),
        ("maintainer", false),
        // Root holds everything by configuration, without any grant at all —
        // which is the property being tested.
        ("breakglass", false),
    ] {
        let user = repo
            .create_user(name, &hash, true)
            .await
            .unwrap_or_else(|e| panic!("create {name}: {e}"));
        if admin {
            repo.grant_user_role(user.id, "admin")
                .await
                .unwrap_or_else(|e| panic!("grant admin to {name}: {e}"));
        }
        let session = repo
            .create_session(user.id, "2099-12-31T23:59:59")
            .await
            .unwrap_or_else(|e| panic!("session {name}: {e}"));
        tokens.push((name, user.id, session.token));
    }

    let user_manager_id = tokens[2].1;
    repo.grant_user_role(user_manager_id, "user_manager")
        .await
        .expect("grant user_manager");

    // The maintainer is responsible for `orders` and nothing else.
    repo.ensure_service("orders").await.expect("orders");
    repo.ensure_service("billing").await.expect("billing");
    let maintainer_id = tokens[3].1;
    let orders_id = repo
        .find_service("orders")
        .await
        .expect("lookup")
        .expect("orders exists");
    repo.add_user_maintainer(orders_id, maintainer_id)
        .await
        .expect("assign maintainer");

    let app = create_app(test_state(repo.clone()));
    World {
        app,
        repo,
        admin: tokens[0].2.clone(),
        plain: tokens[1].2.clone(),
        user_manager: tokens[2].2.clone(),
        maintainer: tokens[3].2.clone(),
        root: tokens[4].2.clone(),
    }
}

#[cfg(test)]
async fn status(app: &axum::Router, method: &str, uri: &str, token: &str) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", token))
        .header("X-CSRF-Token", TEST_CSRF_TOKEN)
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .expect("request");
    app.clone()
        .oneshot(request)
        .await
        .expect("response")
        .status()
}

/// An account that could reach a route before the migration must still reach it.
#[tokio::test]
async fn an_administrator_still_reaches_every_admin_surface() {
    let w = world().await;
    for (method, uri) in [
        ("GET", "/admin/users"),
        ("GET", "/admin/roles"),
        ("GET", "/admin/groups"),
        ("GET", "/admin/protected-branches"),
        ("GET", "/admin/producers"),
        ("GET", "/admin/consumers"),
        ("GET", "/admin/observability/stats"),
        ("GET", "/admin/observability/audit-logs"),
        ("GET", "/admin/settings/dev-mode"),
        ("GET", "/admin/producers/orders/maintainers"),
    ] {
        let got = status(&w.app, method, uri, &w.admin).await;
        assert_ne!(got, StatusCode::FORBIDDEN, "admin refused {method} {uri}");
        assert_ne!(
            got,
            StatusCode::UNAUTHORIZED,
            "admin unauthenticated for {method} {uri}"
        );
    }
}

/// The whole point of the permission model: a partial administrator exists.
#[tokio::test]
async fn a_user_manager_reaches_user_administration_and_nothing_else() {
    let w = world().await;

    for uri in ["/admin/users", "/admin/roles", "/admin/groups"] {
        assert_eq!(
            status(&w.app, "GET", uri, &w.user_manager).await,
            StatusCode::OK,
            "user_manager should reach {uri}"
        );
    }

    for uri in [
        "/admin/protected-branches",
        "/admin/settings/dev-mode",
        "/admin/observability/audit-logs",
        "/admin/auth-config",
    ] {
        assert_eq!(
            status(&w.app, "GET", uri, &w.user_manager).await,
            StatusCode::FORBIDDEN,
            "user_manager should be refused {uri}"
        );
    }
}

#[tokio::test]
async fn a_plain_user_reaches_only_the_read_only_listings() {
    let w = world().await;

    for uri in ["/admin/producers", "/admin/consumers"] {
        assert_eq!(
            status(&w.app, "GET", uri, &w.plain).await,
            StatusCode::OK,
            "a signed-in user should reach {uri}"
        );
    }

    for uri in [
        "/admin/users",
        "/admin/roles",
        "/admin/protected-branches",
        "/admin/settings/dev-mode",
    ] {
        assert_eq!(
            status(&w.app, "GET", uri, &w.plain).await,
            StatusCode::FORBIDDEN,
            "a plain user should be refused {uri}"
        );
    }
}

/// The scoped arm: the same permission, admitted only for the Producers the
/// caller is actually responsible for.
#[tokio::test]
async fn a_maintainer_acts_on_their_producer_and_no_other() {
    let w = world().await;

    assert_ne!(
        status(
            &w.app,
            "DELETE",
            "/admin/producers/orders/branches/main",
            &w.maintainer
        )
        .await,
        StatusCode::FORBIDDEN,
        "the maintainer of orders should not be refused"
    );

    assert_eq!(
        status(
            &w.app,
            "DELETE",
            "/admin/producers/billing/branches/main",
            &w.maintainer
        )
        .await,
        StatusCode::FORBIDDEN,
        "the maintainer of orders must not act on billing"
    );

    // Maintainership confers the maintainer bundle, not administration at large.
    assert_eq!(
        status(&w.app, "GET", "/admin/users", &w.maintainer).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn maintainership_through_a_group_is_honoured_by_the_router() {
    let w = world().await;
    let plain_user = w
        .repo
        .find_user("plain")
        .await
        .expect("lookup")
        .expect("plain exists");
    let group = w
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");
    w.repo
        .add_group_member(group.id, plain_user.id)
        .await
        .expect("member");
    let billing = w
        .repo
        .find_service("billing")
        .await
        .expect("lookup")
        .expect("billing exists");
    w.repo
        .add_group_maintainer(billing, group.id)
        .await
        .expect("assign");

    assert_ne!(
        status(
            &w.app,
            "DELETE",
            "/admin/producers/billing/branches/main",
            &w.plain
        )
        .await,
        StatusCode::FORBIDDEN,
        "membership of a maintaining group should be enough"
    );
}

/// Root's authority comes from configuration, so it holds regardless of the
/// administrator flag or any stored grant.
#[tokio::test]
async fn root_reaches_everything_without_a_grant() {
    let w = world().await;
    assert!(
        w.repo
            .effective_stored_roles(
                w.repo
                    .find_user("breakglass")
                    .await
                    .expect("lookup")
                    .expect("exists")
                    .id
            )
            .await
            .expect("roles")
            .is_empty(),
        "root must hold no stored grant, or this proves nothing"
    );

    for uri in [
        "/admin/users",
        "/admin/roles",
        "/admin/settings/dev-mode",
        "/admin/auth-config",
        "/admin/observability/audit-logs",
    ] {
        assert_ne!(
            status(&w.app, "GET", uri, &w.root).await,
            StatusCode::FORBIDDEN,
            "root should reach {uri}"
        );
    }

    assert_ne!(
        status(
            &w.app,
            "DELETE",
            "/admin/producers/billing/branches/main",
            &w.root
        )
        .await,
        StatusCode::FORBIDDEN,
        "root maintains every Producer"
    );
}

#[tokio::test]
async fn an_unauthenticated_caller_is_refused_everywhere() {
    let w = world().await;
    for uri in ["/admin/users", "/admin/producers", "/admin/roles"] {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = w.app.clone().oneshot(request).await.expect("response");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "anonymous should be refused {uri}"
        );
    }
}

/// The destructive surface is the one where a mistaken `Authenticated` would
/// cost the most, so it is asserted rather than left to the source lint.
#[tokio::test]
async fn destructive_operations_need_their_permission() {
    let w = world().await;
    for uri in ["/admin/nuke/producers", "/admin/nuke/users"] {
        assert_eq!(
            status(&w.app, "POST", uri, &w.plain).await,
            StatusCode::FORBIDDEN,
            "a plain user must not reach {uri}"
        );
        assert_eq!(
            status(&w.app, "POST", uri, &w.user_manager).await,
            StatusCode::FORBIDDEN,
            "a user manager must not reach {uri}"
        );
    }
}

// --- Onboarding toggle ---

/// The onboarding toggle is the one Producer-scoped route a maintainer most
/// needs, and it guards a different permission from the branch routes above, so
/// the scoped arm is asserted for it specifically.
#[tokio::test]
async fn a_maintainer_may_toggle_onboarding_on_their_producer_only() {
    let w = world().await;

    assert_ne!(
        status(
            &w.app,
            "PUT",
            "/admin/producers/orders/onboarding",
            &w.maintainer
        )
        .await,
        StatusCode::FORBIDDEN,
        "the maintainer of orders should be able to set its onboarding"
    );

    assert_eq!(
        status(
            &w.app,
            "PUT",
            "/admin/producers/billing/onboarding",
            &w.maintainer
        )
        .await,
        StatusCode::FORBIDDEN,
        "the maintainer of orders must not touch billing"
    );
}

#[tokio::test]
async fn a_user_without_the_permission_cannot_toggle_onboarding() {
    let w = world().await;
    for token in [&w.plain, &w.user_manager] {
        assert_eq!(
            status(&w.app, "PUT", "/admin/producers/orders/onboarding", token).await,
            StatusCode::FORBIDDEN,
        );
    }
}

#[tokio::test]
async fn toggling_onboarding_takes_effect_and_is_audited() {
    let w = world().await;

    let request = Request::builder()
        .method("PUT")
        .uri("/admin/producers/orders/onboarding")
        .header("Authorization", format!("Bearer {}", w.maintainer))
        .header("X-CSRF-Token", TEST_CSRF_TOKEN)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"onboarding": true}"#))
        .expect("request");
    let response = w.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        w.repo.list_onboarding_producers().await.expect("listing"),
        vec!["orders".to_string()]
    );

    let logs = w.repo.get_recent_audit_logs(50).await.expect("audit logs");
    let entry = logs
        .iter()
        .find(|e| e.action == "START_ONBOARDING")
        .expect("the toggle should be audited");
    assert_eq!(entry.username, "maintainer");
    assert_eq!(entry.service.as_deref(), Some("orders"));
}

// --- What the UI is told ---

/// The UI gates on permissions, so `/auth/me` has to report them. Without this
/// the frontend would have to re-derive the role bundles and could drift from
/// what the server enforces.
#[tokio::test]
async fn the_current_actor_endpoint_reports_roles_and_permissions() {
    let w = world().await;

    let body = |token: String| {
        let app = w.app.clone();
        async move {
            let request = Request::builder()
                .method("GET")
                .uri("/auth/me")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .expect("request");
            let response = app.oneshot(request).await.expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")
        }
    };

    let manager = body(w.user_manager.clone()).await;
    assert_eq!(manager["roles"], serde_json::json!(["user_manager"]));
    let permissions = manager["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .filter_map(|p| p.as_str())
        .collect::<Vec<_>>();
    assert!(permissions.contains(&"manage_users"));
    assert!(permissions.contains(&"manage_roles"));
    assert!(
        !permissions.contains(&"manage_settings"),
        "a user manager must not be told they may change settings"
    );
    assert_eq!(manager["is_root"], false);

    let plain = body(w.plain.clone()).await;
    assert_eq!(plain["roles"], serde_json::json!([]));
    assert_eq!(plain["permissions"], serde_json::json!([]));

    let root = body(w.root.clone()).await;
    assert_eq!(root["is_root"], true);
    assert_eq!(
        root["permissions"]
            .as_array()
            .expect("permissions array")
            .len(),
        sanshain_service::domain::permissions::Permission::ALL.len(),
        "root holds every permission"
    );
}

/// The user list drives the role badges on the management page.
#[tokio::test]
async fn the_user_listing_carries_each_users_roles() {
    let w = world().await;
    let request = Request::builder()
        .method("GET")
        .uri("/admin/users")
        .header("Authorization", format!("Bearer {}", w.admin))
        .body(Body::empty())
        .expect("request");
    let response = w.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let users: serde_json::Value = serde_json::from_slice(&bytes).expect("json");

    let manager = users
        .as_array()
        .expect("array")
        .iter()
        .find(|u| u["username"] == "user_manager")
        .expect("user_manager listed");
    assert_eq!(manager["roles"], serde_json::json!(["user_manager"]));
}

// --- The review inbox ---

#[cfg(test)]
async fn hold_a_spec(repo: &SqliteSpecRepository, producer: &str) -> i64 {
    use sanshain_service::application::spec_service;
    use sanshain_service::domain::models::{ApiType, AppError};

    const V1: &str = "openapi: 3.0.0\ninfo: { title: T, version: 1.0.0 }\npaths:\n  /a:\n    get:\n      responses:\n        \"200\": { description: ok }\n  /b:\n    get:\n      responses:\n        \"200\": { description: ok }\n";
    const V2: &str = "openapi: 3.0.0\ninfo: { title: T, version: 1.0.0 }\npaths:\n  /a:\n    get:\n      responses:\n        \"200\": { description: ok }\n";

    spec_service::provide_spec(repo, producer, "master", ApiType::OpenApi, V1, None, false)
        .await
        .expect("first provide");
    let err =
        spec_service::provide_spec(repo, producer, "master", ApiType::OpenApi, V2, None, false)
            .await
            .expect_err("breaking change refused");
    match err {
        AppError::Quarantined { pending_id, .. } => pending_id,
        other => panic!("expected the submission to be held, got {other:?}"),
    }
}

/// The inbox shows an administrator everything and a maintainer only theirs —
/// filtered rather than refused, so the page opens for both.
#[tokio::test]
async fn the_inbox_shows_only_what_the_caller_may_review() {
    let w = world().await;
    hold_a_spec(&w.repo, "orders").await;
    hold_a_spec(&w.repo, "billing").await;

    let read = |token: String| {
        let app = w.app.clone();
        async move {
            let request = Request::builder()
                .method("GET")
                .uri("/admin/pending-specs")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .expect("request");
            let response = app.oneshot(request).await.expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")
        }
    };

    let as_admin = read(w.admin.clone()).await;
    assert_eq!(
        as_admin["count"], 2,
        "an administrator sees every held spec"
    );

    let as_maintainer = read(w.maintainer.clone()).await;
    assert_eq!(as_maintainer["count"], 1);
    assert_eq!(as_maintainer["pending"][0]["producer"], "orders");

    let as_plain = read(w.plain.clone()).await;
    assert_eq!(
        as_plain["count"], 0,
        "someone with no review rights gets an empty inbox, not an error"
    );
}

#[tokio::test]
async fn accepting_applies_the_held_spec_and_clears_it() {
    let w = world().await;
    let pending_id = hold_a_spec(&w.repo, "orders").await;

    let service = w
        .repo
        .find_service("orders")
        .await
        .expect("lookup")
        .expect("orders");
    assert!(
        w.repo
            .find_endpoint(
                service,
                "master",
                sanshain_service::domain::models::ApiType::OpenApi,
                "/b",
                "GET"
            )
            .await
            .expect("lookup")
            .is_some(),
        "the endpoint the held spec removes is still live before acceptance"
    );

    assert_eq!(
        status(
            &w.app,
            "POST",
            &format!("/admin/pending-specs/{}/accept", pending_id),
            &w.maintainer
        )
        .await,
        StatusCode::OK,
        "the maintainer of orders may accept it"
    );

    assert!(
        w.repo
            .find_endpoint(
                service,
                "master",
                sanshain_service::domain::models::ApiType::OpenApi,
                "/b",
                "GET"
            )
            .await
            .expect("lookup")
            .is_none(),
        "accepting must actually apply the change"
    );
    assert!(
        w.repo
            .list_pending_specs()
            .await
            .expect("listing")
            .is_empty(),
        "the entry is cleared once applied"
    );
}

#[tokio::test]
async fn rejecting_clears_the_entry_without_applying_it() {
    let w = world().await;
    let pending_id = hold_a_spec(&w.repo, "orders").await;

    assert_eq!(
        status(
            &w.app,
            "DELETE",
            &format!("/admin/pending-specs/{}", pending_id),
            &w.admin
        )
        .await,
        StatusCode::OK
    );

    let service = w
        .repo
        .find_service("orders")
        .await
        .expect("lookup")
        .expect("orders");
    assert!(
        w.repo
            .find_endpoint(
                service,
                "master",
                sanshain_service::domain::models::ApiType::OpenApi,
                "/b",
                "GET"
            )
            .await
            .expect("lookup")
            .is_some(),
        "rejecting must apply nothing"
    );
    assert!(
        w.repo
            .list_pending_specs()
            .await
            .expect("listing")
            .is_empty()
    );
}

#[tokio::test]
async fn a_maintainer_cannot_act_on_another_producers_held_spec() {
    let w = world().await;
    let billing = hold_a_spec(&w.repo, "billing").await;

    for (method, uri) in [
        ("POST", format!("/admin/pending-specs/{}/accept", billing)),
        ("DELETE", format!("/admin/pending-specs/{}", billing)),
        ("GET", format!("/admin/pending-specs/{}", billing)),
    ] {
        assert_eq!(
            status(&w.app, method, &uri, &w.maintainer).await,
            StatusCode::FORBIDDEN,
            "the maintainer of orders must not reach {method} {uri}"
        );
    }
    assert_eq!(
        w.repo.list_pending_specs().await.expect("listing").len(),
        1,
        "nothing was applied or discarded"
    );
}

#[tokio::test]
async fn review_decisions_are_audited() {
    let w = world().await;
    let accepted = hold_a_spec(&w.repo, "orders").await;
    status(
        &w.app,
        "POST",
        &format!("/admin/pending-specs/{}/accept", accepted),
        &w.maintainer,
    )
    .await;

    let rejected = hold_a_spec(&w.repo, "billing").await;
    status(
        &w.app,
        "DELETE",
        &format!("/admin/pending-specs/{}", rejected),
        &w.admin,
    )
    .await;

    let logs = w.repo.get_recent_audit_logs(100).await.expect("audit logs");
    assert!(
        logs.iter()
            .any(|e| e.action == "ACCEPTED_PENDING" && e.username == "maintainer"),
        "the acceptance is attributed to whoever made it"
    );
    assert!(
        logs.iter()
            .any(|e| e.action == "REJECTED_PENDING" && e.username == "admin"),
        "so is the rejection"
    );
}

#[tokio::test]
async fn acting_on_an_unknown_entry_is_not_found() {
    let w = world().await;
    assert_eq!(
        status(&w.app, "POST", "/admin/pending-specs/9999/accept", &w.admin).await,
        StatusCode::NOT_FOUND
    );
}

// --- Maintainers listing: readable by that Producer's maintainers ---

/// The GET was split from the POST so a maintainer can see who shares
/// responsibility for their Producer, while changing the assignment stays
/// admin-only. Both directions are asserted, and the split itself is exercised:
/// axum merges two `.route()` calls for one path at router build time, so these
/// requests also prove the merge holds.
#[tokio::test]
async fn a_maintainer_may_read_their_own_producers_maintainers_but_not_assign() {
    let w = world().await;

    assert_eq!(
        status(
            &w.app,
            "GET",
            "/admin/producers/orders/maintainers",
            &w.maintainer
        )
        .await,
        StatusCode::OK,
        "the maintainer of orders may see who maintains orders"
    );

    assert_eq!(
        status(
            &w.app,
            "GET",
            "/admin/producers/billing/maintainers",
            &w.maintainer
        )
        .await,
        StatusCode::FORBIDDEN,
        "but not who maintains billing"
    );

    // Assignment is still who-may-do-what work: admin (ManageRoles), not scope.
    let request = Request::builder()
        .method("POST")
        .uri("/admin/producers/orders/maintainers")
        .header("Authorization", format!("Bearer {}", w.maintainer))
        .header("X-CSRF-Token", TEST_CSRF_TOKEN)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"user_id": 1}"#))
        .expect("request");
    let response = w.app.clone().oneshot(request).await.expect("response");
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a maintainer must not assign maintainers, even on their own Producer"
    );

    // The admin still reaches both methods.
    assert_eq!(
        status(
            &w.app,
            "GET",
            "/admin/producers/orders/maintainers",
            &w.admin
        )
        .await,
        StatusCode::OK
    );
}

// --- Endpoint editing: a maintainer's own work ---

/// Editing a Producer's spec admits the maintainer of that Producer. The
/// Producer is named in the request *body*, so the scoped check lives in the
/// handler rather than the route guard — these tests pin that it actually runs.
#[tokio::test]
async fn a_maintainer_may_edit_their_own_producers_endpoints_and_no_others() {
    let w = world().await;

    // Give both Producers a real endpoint on a feature branch.
    const SPEC: &str = "openapi: 3.0.0\ninfo: { title: T, version: 1.0.0 }\npaths:\n  /a:\n    get:\n      responses:\n        \"200\": { description: ok }\n";
    for producer in ["orders", "billing"] {
        sanshain_service::application::spec_service::provide_spec(
            &w.repo,
            producer,
            "dev",
            sanshain_service::domain::models::ApiType::OpenApi,
            SPEC,
            None,
            false,
        )
        .await
        .expect("provide");
    }

    let edit = |token: String, producer: &'static str| {
        let app = w.app.clone();
        async move {
            let payload = serde_json::json!({
                "producername": producer,
                "branch": "dev",
                "api_type": "openapi",
                "path": "/a",
                "method": "GET",
                "yaml": "get:\n  responses:\n    \"200\": { description: edited }\n",
                "deprecated": true,
                "external": false,
            });
            let request = Request::builder()
                .method("POST")
                .uri("/admin/endpoints/update")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Content-Type", "application/json")
                .body(Body::from(payload.to_string()))
                .expect("request");
            app.oneshot(request).await.expect("response").status()
        }
    };

    assert_eq!(
        edit(w.maintainer.clone(), "orders").await,
        StatusCode::OK,
        "the maintainer of orders edits orders"
    );
    assert_eq!(
        edit(w.maintainer.clone(), "billing").await,
        StatusCode::FORBIDDEN,
        "but not billing"
    );
    assert_eq!(
        edit(w.plain.clone(), "orders").await,
        StatusCode::FORBIDDEN,
        "a plain user edits nothing"
    );
    assert_eq!(
        edit(w.admin.clone(), "billing").await,
        StatusCode::OK,
        "an administrator still edits anything"
    );

    // The maintainer's edit actually landed.
    let service = w
        .repo
        .find_service("orders")
        .await
        .expect("lookup")
        .expect("orders");
    let (_, _, deprecated, _) = w
        .repo
        .find_endpoint(
            service,
            "dev",
            sanshain_service::domain::models::ApiType::OpenApi,
            "/a",
            "GET",
        )
        .await
        .expect("lookup")
        .expect("endpoint");
    assert!(deprecated, "the edit was applied, not just authorised");
}

/// The editor UI decides what to offer from `/auth/me`, so the maintained
/// Producers have to be reported there.
#[tokio::test]
async fn auth_me_reports_maintained_producers() {
    let w = world().await;
    let request = Request::builder()
        .method("GET")
        .uri("/auth/me")
        .header("Authorization", format!("Bearer {}", w.maintainer))
        .body(Body::empty())
        .expect("request");
    let response = w.app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let me: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(me["maintains"], serde_json::json!(["orders"]));
}

/// The aggregate behind the dashboard's Maintainers section: one response for
/// every Producer, so the page stops asking per Producer.
#[tokio::test]
async fn the_maintainers_aggregate_answers_for_every_producer_at_once() {
    let w = world().await;
    let group = w
        .repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");
    let billing = w
        .repo
        .find_service("billing")
        .await
        .expect("lookup")
        .expect("billing");
    w.repo
        .add_group_maintainer(billing, group.id)
        .await
        .expect("assign");

    // A Producer only appears in listings once it has a branch, so give both
    // one — the fixture registers the services bare.
    for name in ["orders", "billing"] {
        let sid = w
            .repo
            .find_service(name)
            .await
            .expect("lookup")
            .expect("exists");
        w.repo.ensure_branch(sid, "master").await.expect("branch");
    }

    let read = |token: String| {
        let app = w.app.clone();
        async move {
            let request = Request::builder()
                .method("GET")
                .uri("/admin/maintainers")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .expect("request");
            app.oneshot(request).await.expect("response")
        }
    };

    let response = read(w.admin.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let data: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let producers = data["producers"].as_array().expect("array");
    assert_eq!(
        producers.len(),
        2,
        "every Producer appears, so the same payload fills the picker"
    );

    let by_name = |name: &str| {
        producers
            .iter()
            .find(|p| p["producer"] == name)
            .unwrap_or_else(|| panic!("{name} listed"))
            .clone()
    };
    let maintainer_id = w
        .repo
        .find_user("maintainer")
        .await
        .expect("lookup")
        .expect("exists")
        .id;
    assert_eq!(
        by_name("orders")["user_ids"],
        serde_json::json!([maintainer_id])
    );
    assert_eq!(
        by_name("billing")["group_ids"],
        serde_json::json!([group.id])
    );

    // Who-may-do-what work: user managers read it, maintainers and plain users
    // do not.
    assert_eq!(read(w.user_manager.clone()).await.status(), StatusCode::OK);
    assert_eq!(
        read(w.maintainer.clone()).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(read(w.plain.clone()).await.status(), StatusCode::FORBIDDEN);
}
