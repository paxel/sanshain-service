//! san-1018 against any repository: a trunk require or bundle to a Producer
//! replaces the Consumer's dependencies on that Producer's API family, in the
//! dev graph and the main graph; plain requires and other Producers are left
//! alone; the Remove action drops every dependency on one Producer.

use sanshain_service::application::admin_service;
use sanshain_service::application::require_service::{
    self, RequireBundleParams, RequireEndpointParams, RequireFormat,
};
use sanshain_service::application::spec_service::{self, ProvideSpecParams};
use sanshain_service::domain::models::{ApiType, SemVer, Stability};
use sanshain_service::domain::permissions::Actor;
use sanshain_service::domain::ports::SpecRepository;

const RESPONSES: &str = "      responses:\n        '200':\n          description: OK\n";

fn openapi(title: &str, version: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo:\n  title: {title}\n  version: {version}\npaths:\n  /a:\n    get:\n{RESPONSES}  /b:\n    get:\n{RESPONSES}  /c/{{id}}:\n    get:\n{RESPONSES}"
    )
}

const ASYNCAPI: &str = "asyncapi: 2.6.0\ninfo:\n  title: Events\n  version: 1.0.0\nchannels:\n  orders.created:\n    publish:\n      message:\n        name: OrderCreated\n        payload:\n          type: object\n";

async fn provide(repo: &impl SpecRepository, producer: &str, api_type: ApiType, content: &str) {
    spec_service::provide_spec(
        repo,
        ProvideSpecParams {
            producername: producer,
            api_type,
            content,
            stability: Stability::Snapshot,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(Actor::test_caller()),
            require_prior_content_match: false,
        },
    )
    .await
    .unwrap();
}

async fn require_one(
    repo: &impl SpecRepository,
    producer: &str,
    version: SemVer,
    path: &str,
    trunk: bool,
) {
    require_service::require_endpoint(
        repo,
        RequireEndpointParams {
            consumername: "shop",
            producername: producer,
            version,
            api_type: ApiType::OpenApi,
            path,
            method: "GET",
            trunk,
            tag: None,
            format: RequireFormat::Native,
            requires_hash: None,
        },
    )
    .await
    .unwrap();
}

async fn require_bundle(repo: &impl SpecRepository, producer: &str, paths: &[&str], trunk: bool) {
    let endpoints: Vec<(String, String)> = paths
        .iter()
        .map(|p| ((*p).to_string(), "GET".to_string()))
        .collect();
    require_service::require_bundle(
        repo,
        RequireBundleParams {
            consumername: "shop",
            producername: producer,
            version: SemVer::new(1, 0, 0),
            api_type: ApiType::OpenApi,
            endpoints: &endpoints,
            trunk,
            tag: None,
            format: RequireFormat::Native,
            requires_hash: None,
        },
    )
    .await
    .unwrap();
}

/// The dev graph of `shop`: `(producer, api type, version, path)`, sorted.
async fn dev(repo: &impl SpecRepository) -> Vec<(String, ApiType, String, String)> {
    let mut rows: Vec<_> = repo
        .list_consumer_endpoints("shop")
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.service, e.api_type, e.version.to_string(), e.path))
        .collect();
    rows.sort();
    rows
}

/// The main graph of `shop`: `(producer, api type, version, path)`, sorted.
async fn main(repo: &impl SpecRepository) -> Vec<(String, ApiType, String, String)> {
    let mut rows: Vec<_> = repo
        .list_current_trunk_pins()
        .await
        .unwrap()
        .into_iter()
        .filter(|p| p.client == "shop")
        .map(|p| (p.service, p.api_type, p.version.to_string(), p.path))
        .collect();
    rows.sort();
    rows
}

fn row(
    producer: &str,
    api_type: ApiType,
    version: &str,
    path: &str,
) -> (String, ApiType, String, String) {
    (
        producer.to_string(),
        api_type,
        version.to_string(),
        path.to_string(),
    )
}

pub async fn trunk_requires_replace_the_dependencies_on_their_producer(repo: &impl SpecRepository) {
    provide(repo, "users", ApiType::OpenApi, &openapi("Users", "1.0.0")).await;
    provide(repo, "users", ApiType::OpenApi, &openapi("Users", "2.0.0")).await;
    provide(repo, "users", ApiType::AsyncApi, ASYNCAPI).await;
    provide(
        repo,
        "orders",
        ApiType::OpenApi,
        &openapi("Orders", "1.0.0"),
    )
    .await;

    // A trunk bundle on users, a trunk require on orders, a dev-only AsyncAPI
    // require on users.
    require_bundle(repo, "users", &["/a", "/b", "/c/{id}"], true).await;
    require_one(repo, "orders", SemVer::new(1, 0, 0), "/a", true).await;
    require_service::require_endpoint(
        repo,
        RequireEndpointParams {
            consumername: "shop",
            producername: "users",
            version: SemVer::new(1, 0, 0),
            api_type: ApiType::AsyncApi,
            path: "orders.created",
            method: "PUB",
            trunk: false,
            tag: None,
            format: RequireFormat::Native,
            requires_hash: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(main(repo).await.len(), 4);
    assert_eq!(dev(repo).await.len(), 5);

    // A plain require kills nothing.
    require_one(repo, "users", SemVer::new(1, 0, 0), "/a", false).await;
    assert_eq!(main(repo).await.len(), 4);
    assert_eq!(dev(repo).await.len(), 5);

    // A trunk bundle keeps only its endpoints on users' OpenAPI; orders and
    // users' AsyncAPI are untouched.
    require_bundle(repo, "users", &["/a", "/c/{id}"], true).await;
    assert_eq!(
        main(repo).await,
        vec![
            row("orders", ApiType::OpenApi, "1.0.0", "/a"),
            row("users", ApiType::OpenApi, "1.0.0", "/a"),
            row("users", ApiType::OpenApi, "1.0.0", "/c/{id}"),
        ]
    );
    assert_eq!(
        dev(repo).await,
        vec![
            row("orders", ApiType::OpenApi, "1.0.0", "/a"),
            row("users", ApiType::OpenApi, "1.0.0", "/a"),
            row("users", ApiType::OpenApi, "1.0.0", "/c/{id}"),
            row("users", ApiType::AsyncApi, "1.0.0", "orders.created"),
        ]
    );

    // A single trunk require at a new Pin leaves only its endpoint alive, at
    // its version.
    require_one(repo, "users", SemVer::new(2, 0, 0), "/b", true).await;
    assert_eq!(
        main(repo).await,
        vec![
            row("orders", ApiType::OpenApi, "1.0.0", "/a"),
            row("users", ApiType::OpenApi, "2.0.0", "/b"),
        ]
    );
    assert_eq!(
        dev(repo).await,
        vec![
            row("orders", ApiType::OpenApi, "1.0.0", "/a"),
            row("users", ApiType::OpenApi, "2.0.0", "/b"),
            row("users", ApiType::AsyncApi, "1.0.0", "orders.created"),
        ]
    );

    // Remove drops every dependency on users, both API types, both graphs.
    let removed = admin_service::remove_consumer_dependencies(repo, "shop", "users")
        .await
        .unwrap();
    assert_eq!(removed, 3);
    assert_eq!(
        main(repo).await,
        vec![row("orders", ApiType::OpenApi, "1.0.0", "/a")]
    );
    assert_eq!(
        dev(repo).await,
        vec![row("orders", ApiType::OpenApi, "1.0.0", "/a")]
    );

    // Nothing left to remove is a 404.
    let again = admin_service::remove_consumer_dependencies(repo, "shop", "users").await;
    assert!(
        matches!(
            again,
            Err(sanshain_service::domain::models::AppError::NotFound(_))
        ),
        "{again:?}"
    );
}
