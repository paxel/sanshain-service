//! Products (#76, ADR-0011) at the router seam: real SQLite repository, real
//! users, roles and groups, real seeded CSRF token — no bypasses. The
//! repository round trip runs here on SQLite; its Postgres run lives in
//! `postgres_test.rs`.

#[path = "common/product_membership.rs"]
mod product_membership;

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
use serde_json::{Value, json};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tower::ServiceExt;

const TEST_CSRF_TOKEN: &str = "products-test-csrf-token";

static TEST_PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
    OnceLock::new();

#[tokio::test]
async fn sqlite_round_trips_product_membership() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    product_membership::product_membership_round_trips(&repo).await;
}

// `cfg(test)` is always true in this crate; the attribute marks these helpers
// as test code so clippy's `allow-*-in-tests` exemptions apply to them.
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
struct Fixture {
    app: axum::Router,
    repo: SqliteSpecRepository,
    /// An `admin`: holds `manage_producers` and `manage_roles` globally.
    admin: String,
    /// A user without any role, made a Product Maintainer in the tests.
    pm_id: i64,
    pm: String,
    /// A user without any role and without any grant.
    plain: String,
}

#[cfg(test)]
async fn session_for(repo: &SqliteSpecRepository, username: &str) -> (i64, String) {
    let hash = services::hash_password("pass-word-1").unwrap();
    let user = repo.create_user(username, &hash, true).await.unwrap();
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();
    (user.id, session.token)
}

/// Systems: `orders` and `billing` are Producers, `webapp` is a Consumer
/// (and requires `orders`), `search` is a Producer outside every Product.
#[cfg(test)]
async fn fixture() -> Fixture {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    let (admin_id, admin) = session_for(&repo, "admin").await;
    repo.grant_user_role(admin_id, "admin").await.unwrap();
    let (pm_id, pm) = session_for(&repo, "pm").await;
    let (_, plain) = session_for(&repo, "plain").await;
    let app = create_app(test_state(repo.clone()));
    let f = Fixture {
        app,
        repo,
        admin,
        pm_id,
        pm,
        plain,
    };
    for producer in ["orders", "billing", "search"] {
        provide(&f, producer).await;
    }
    let (status, body) = call(
        &f,
        "GET",
        "/require?consumername=webapp&producername=orders&version=1.0.0&path=/users&method=GET",
        &f.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "require failed: {body}");
    f
}

#[cfg(test)]
async fn provide(f: &Fixture, producer: &str) {
    let spec = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n";
    let (status, body) = call(
        f,
        "POST",
        "/provide",
        &f.admin,
        Some(json!({"producername": producer, "openapi_yaml": spec, "stability": "snapshot"})),
    )
    .await;
    assert!(status.is_success(), "provide failed: {status} {body}");
}

#[cfg(test)]
async fn call(
    f: &Fixture,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"));
    if method != "GET" {
        builder = builder.header("X-CSRF-Token", TEST_CSRF_TOKEN);
    }
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(Body::from(json.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = f.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
    (status, value)
}

#[cfg(test)]
async fn create(f: &Fixture, name: &str) {
    let (status, body) = call(
        f,
        "POST",
        "/admin/products",
        &f.admin,
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[cfg(test)]
async fn import(f: &Fixture, product: &str, token: &str, chart: &str) -> (StatusCode, Value) {
    call(
        f,
        "POST",
        &format!("/admin/products/{product}/chart"),
        token,
        Some(json!({"chart_yaml": chart})),
    )
    .await
}

/// `(system, origin, producer, consumer)` of every member, as the page gets it.
#[cfg(test)]
async fn members(f: &Fixture, product: &str) -> Vec<(String, String, bool, bool)> {
    let (status, body) = call(
        f,
        "GET",
        &format!("/admin/products/{product}"),
        &f.plain,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["system"].as_str().unwrap().to_string(),
                m["origin"].as_str().unwrap().to_string(),
                m["producer"].as_bool().unwrap(),
                m["consumer"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[cfg(test)]
fn member(
    system: &str,
    origin: &str,
    producer: bool,
    consumer: bool,
) -> (String, String, bool, bool) {
    (system.to_string(), origin.to_string(), producer, consumer)
}

#[cfg(test)]
async fn audit(f: &Fixture) -> Vec<(String, String, String)> {
    let (status, body) = call(
        f,
        "GET",
        "/admin/observability/audit-logs?limit=200",
        &f.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["action"].as_str().unwrap().to_string(),
                e["username"].as_str().unwrap().to_string(),
                e["details"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
fn audited(entries: &[(String, String, String)], action: &str, user: &str, details: &str) -> bool {
    entries
        .iter()
        .any(|(a, u, d)| a == action && u == user && d == details)
}

#[tokio::test]
async fn products_are_created_listed_described_and_deleted_without_touching_systems() {
    let f = fixture().await;
    create(&f, "shop").await;
    // A Product may carry a system's name; names are unique among Products.
    create(&f, "orders").await;
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products",
        &f.admin,
        Some(json!({"name": "shop"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products",
        &f.admin,
        Some(json!({"name": "a/b"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/members",
        &f.admin,
        Some(json!({"system": "orders"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = call(&f, "GET", "/admin/products", &f.plain, None).await;
    assert_eq!(status, StatusCode::OK);
    let listed: Vec<(String, u64)> = body["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_string(),
                p["member_count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(listed, [("orders".to_string(), 0), ("shop".to_string(), 1)]);

    let (status, body) = call(&f, "DELETE", "/admin/products/shop", &f.admin, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], "shop");
    let (status, _) = call(&f, "GET", "/admin/products/shop", &f.plain, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&f, "DELETE", "/admin/products/shop", &f.admin, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The envelope went, the letter stays.
    let (status, _) = call(&f, "GET", "/admin/producers/orders", &f.plain, None).await;
    assert_eq!(status, StatusCode::OK);

    let entries = audit(&f).await;
    assert!(audited(
        &entries,
        "CREATE_PRODUCT",
        "admin",
        "Created Product 'shop'"
    ));
    assert!(audited(
        &entries,
        "CREATE_PRODUCT",
        "admin",
        "Created Product 'orders'"
    ));
    assert!(audited(
        &entries,
        "DELETE_PRODUCT",
        "admin",
        "Deleted Product 'shop'"
    ));
    assert!(audited(
        &entries,
        "ADD_PRODUCT_MEMBER",
        "admin",
        "Added 'orders' to Product 'shop' by hand"
    ));
}

#[tokio::test]
async fn members_are_added_by_hand_only_when_known_and_removed_with_an_audit_entry() {
    let f = fixture().await;
    create(&f, "shop").await;
    let add = |system: &'static str| {
        let f = &f;
        async move {
            call(
                f,
                "POST",
                "/admin/products/shop/members",
                &f.admin,
                Some(json!({"system": system})),
            )
            .await
        }
    };
    // A pure Consumer is a system too.
    assert_eq!(add("webapp").await.0, StatusCode::CREATED);
    let (status, body) = add("webapp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["added"], false);
    assert_eq!(add("redis").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(add("orders").await.0, StatusCode::CREATED);
    assert_eq!(
        members(&f, "shop").await,
        [
            member("orders", "manual", true, false),
            member("webapp", "manual", false, true),
        ]
    );

    let (status, _) = call(
        &f,
        "DELETE",
        "/admin/products/shop/members/webapp",
        &f.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &f,
        "DELETE",
        "/admin/products/shop/members/webapp",
        &f.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        members(&f, "shop").await,
        [member("orders", "manual", true, false)]
    );
    assert!(audited(
        &audit(&f).await,
        "REMOVE_PRODUCT_MEMBER",
        "admin",
        "Removed 'webapp' from Product 'shop'"
    ));
}

const FIRST_CHART: &str = r#"apiVersion: v2
name: shop
version: 1.0.0
dependencies:
  - name: orders-service
    alias: orders
    version: 2.0.0
    repository: oci://charts.example.test
  - name: billing
    version: 1.0.0
  - name: redis
    version: 18.0.0
  - name: postgresql
    version: 15.0.0
"#;

#[tokio::test]
async fn a_chart_import_follows_the_current_chart_and_leaves_manual_members_alone() {
    let f = fixture().await;
    create(&f, "shop").await;
    for system in ["webapp", "search"] {
        let (status, _) = call(
            &f,
            "POST",
            "/admin/products/shop/members",
            &f.admin,
            Some(json!({"system": system})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // alias wins over name; infrastructure subcharts are skipped and named.
    let (status, report) = import(&f, "shop", &f.admin, FIRST_CHART).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(
        report,
        json!({"added": ["billing", "orders"], "removed": [], "skipped": ["postgresql", "redis"]})
    );
    assert_eq!(
        members(&f, "shop").await,
        [
            member("billing", "helm", true, false),
            member("orders", "helm", true, false),
            member("search", "manual", true, false),
            member("webapp", "manual", false, true),
        ]
    );

    // Removed by hand, then listed again: the chart brings it back.
    let (status, _) = call(
        &f,
        "DELETE",
        "/admin/products/shop/members/orders",
        &f.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The next chart drops billing and lists webapp, added by hand earlier.
    let second = "dependencies:\n  - name: orders\n  - name: webapp\n";
    let (status, report) = import(&f, "shop", &f.admin, second).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(
        report,
        json!({"added": ["orders"], "removed": ["billing"], "skipped": []})
    );
    assert_eq!(
        members(&f, "shop").await,
        [
            member("orders", "helm", true, false),
            member("search", "manual", true, false),
            member("webapp", "helm", false, true),
        ]
    );
    // webapp is chart-sourced now: a chart without it removes it.
    let (status, report) = import(&f, "shop", &f.admin, "dependencies:\n  - name: orders\n").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["removed"], json!(["webapp"]));

    let entries = audit(&f).await;
    assert!(audited(
        &entries,
        "IMPORT_PRODUCT_CHART",
        "admin",
        "Imported a Chart.yaml into Product 'shop': added billing, orders; removed none; skipped postgresql, redis"
    ));
    assert!(audited(
        &entries,
        "IMPORT_PRODUCT_CHART",
        "admin",
        "Imported a Chart.yaml into Product 'shop': added orders; removed billing; skipped none"
    ));
}

#[tokio::test]
async fn a_bad_chart_is_refused_and_changes_nothing() {
    let f = fixture().await;
    create(&f, "shop").await;
    let (status, _) = import(&f, "shop", &f.admin, FIRST_CHART).await;
    assert_eq!(status, StatusCode::OK);
    let before = members(&f, "shop").await;
    for bad in [
        "dependencies: [unclosed",
        "apiVersion: v2\nname: shop\n",
        "name: shop\ndependencies: []\n",
        "dependencies:\n  - version: 1.0.0\n",
    ] {
        let (status, body) = import(&f, "shop", &f.admin, bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert_eq!(members(&f, "shop").await, before, "{bad} changed members");
    }
    let (status, _) = import(&f, "ghost", &f.admin, FIRST_CHART).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_product_maintainer_edits_only_their_products_membership() {
    let f = fixture().await;
    create(&f, "shop").await;
    create(&f, "other").await;
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/maintainers",
        &f.admin,
        Some(json!({"user_id": f.pm_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Their own Product: add, import, remove.
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/members",
        &f.pm,
        Some(json!({"system": "search"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = import(&f, "shop", &f.pm, FIRST_CHART).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &f,
        "DELETE",
        "/admin/products/shop/members/search",
        &f.pm,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(audited(
        &audit(&f).await,
        "REMOVE_PRODUCT_MEMBER",
        "pm",
        "Removed 'search' from Product 'shop'"
    ));

    // Someone else's Product, and the administrative acts: refused.
    for (method, uri, body) in [
        (
            "POST",
            "/admin/products/other/members",
            Some(json!({"system": "search"})),
        ),
        (
            "POST",
            "/admin/products/other/chart",
            Some(json!({"chart_yaml": FIRST_CHART})),
        ),
        ("DELETE", "/admin/products/shop/members/orders", None),
        ("POST", "/admin/products", Some(json!({"name": "mine"}))),
        ("DELETE", "/admin/products/shop", None),
        (
            "POST",
            "/admin/products/shop/maintainers",
            Some(json!({"user_id": f.pm_id})),
        ),
        (
            "DELETE",
            &format!("/admin/products/shop/maintainers/users/{}", f.pm_id),
            None,
        ),
    ] {
        let token = if uri.ends_with("/members/orders") {
            &f.plain
        } else {
            &f.pm
        };
        let (status, _) = call(&f, method, uri, token, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    // No power over the member systems themselves.
    for (method, uri) in [
        ("DELETE", "/admin/producers/orders"),
        ("DELETE", "/admin/systems/orders/information"),
        ("GET", "/admin/producers/orders/webhooks"),
    ] {
        let (status, _) = call(&f, method, uri, &f.pm, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    // Reading is open to anyone signed in, and names the maintainer.
    let (status, body) = call(&f, "GET", "/admin/products/shop", &f.plain, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["maintainers"]["users"][0]["username"], "pm");
    let (status, me) = call(&f, "GET", "/auth/me", &f.pm, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["maintains_products"], json!(["shop"]));
    assert_eq!(me["maintains"], json!([]));
    assert_eq!(me["permissions"], json!([]));
}

#[tokio::test]
async fn group_maintainers_are_assigned_and_unassigned_with_audit_entries() {
    let f = fixture().await;
    create(&f, "shop").await;
    let team = f
        .repo
        .create_group("team-shop", GroupSource::Native)
        .await
        .unwrap();
    f.repo.add_group_member(team.id, f.pm_id).await.unwrap();
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/maintainers",
        &f.admin,
        Some(json!({"group_id": team.id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/maintainers",
        &f.admin,
        Some(json!({"group_id": team.id, "user_id": f.pm_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = call(&f, "GET", "/admin/products/shop", &f.plain, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["maintainers"],
        json!({"users": [], "groups": [{"id": team.id, "name": "team-shop", "source": "native"}]})
    );
    // Membership of the group is maintainership.
    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/members",
        &f.pm,
        Some(json!({"system": "orders"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let uri = format!("/admin/products/shop/maintainers/groups/{}", team.id);
    let (status, _) = call(&f, "DELETE", &uri, &f.admin, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&f, "DELETE", &uri, &f.admin, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &f,
        "DELETE",
        "/admin/products/shop/members/orders",
        &f.pm,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the team handed the Product over"
    );

    let (status, _) = call(
        &f,
        "POST",
        "/admin/products/shop/maintainers",
        &f.admin,
        Some(json!({"user_id": f.pm_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let uri = format!("/admin/products/shop/maintainers/users/{}", f.pm_id);
    let (status, _) = call(&f, "DELETE", &uri, &f.admin, None).await;
    assert_eq!(status, StatusCode::OK);

    let entries = audit(&f).await;
    let group = team.id;
    let user = f.pm_id;
    assert!(audited(
        &entries,
        "ASSIGN_PRODUCT_MAINTAINER",
        "admin",
        &format!("Group {group} now maintains Product 'shop'")
    ));
    assert!(audited(
        &entries,
        "UNASSIGN_PRODUCT_MAINTAINER",
        "admin",
        &format!("Group {group} no longer maintains Product 'shop'")
    ));
    assert!(audited(
        &entries,
        "ASSIGN_PRODUCT_MAINTAINER",
        "admin",
        &format!("User {user} now maintains Product 'shop'")
    ));
    assert!(audited(
        &entries,
        "UNASSIGN_PRODUCT_MAINTAINER",
        "admin",
        &format!("User {user} no longer maintains Product 'shop'")
    ));
}

#[tokio::test]
async fn a_member_whose_system_is_gone_is_not_in_sanshain_until_it_returns() {
    let f = fixture().await;
    create(&f, "shop").await;
    let (status, _) = import(&f, "shop", &f.admin, FIRST_CHART).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&f, "DELETE", "/admin/producers/billing", &f.admin, None).await;
    assert!(status.is_success(), "{status} {body}");
    // The member stays, with neither role: "not in Sanshain".
    assert_eq!(
        members(&f, "shop").await,
        [
            member("billing", "helm", false, false),
            member("orders", "helm", true, false),
        ]
    );
    // A system of that name provides again: live, without editing the Product.
    provide(&f, "billing").await;
    assert_eq!(
        members(&f, "shop").await,
        [
            member("billing", "helm", true, false),
            member("orders", "helm", true, false),
        ]
    );
}
