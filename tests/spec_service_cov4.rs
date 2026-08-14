//! Service-seam coverage of the spec service's refusal paths against the real
//! SQLite repository: unparsable documents per API type, the AsyncAPI
//! channel-contract ownership rules on GA provides, and the endpoint-history
//! not-found answers.

use sanshain_service::application::spec_service;
use sanshain_service::domain::models::{ApiType, AppError, Stability};
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn setup() -> SqliteSpecRepository {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    repo
}

#[cfg(test)]
fn params<'a>(
    producer: &'a str,
    api_type: ApiType,
    content: &'a str,
    stability: Stability,
) -> spec_service::ProvideSpecParams<'a> {
    spec_service::ProvideSpecParams {
        producername: producer,
        api_type,
        content,
        stability,
        dry_run: false,
        trunk: false,
        tag: None,
        caller: Some(if stability == Stability::Ga {
            sanshain_service::domain::permissions::Actor::test_releaser()
        } else {
            sanshain_service::domain::permissions::Actor::test_caller()
        }),
        require_prior_content_match: false,
    }
}

/// AsyncAPI 2.x document with one named PUB message whose payload `id`
/// property has the given type.
#[cfg(test)]
fn asyncapi_with_id_type(version: &str, id_type: &str) -> String {
    format!(
        r#"asyncapi: 2.6.0
info:
  title: T
  version: {version}
channels:
  user-created:
    publish:
      message:
        name: UserCreated
        payload:
          type: object
          properties:
            id: {{ type: {id_type} }}
"#
    )
}

#[tokio::test]
async fn provide_rejects_unparsable_documents_per_api_type() {
    let repo = setup().await;

    // OpenAPI without an info.version.
    let err = spec_service::provide_spec(
        &repo,
        params(
            "svc",
            ApiType::OpenApi,
            "openapi: 3.0.0\ninfo:\n  title: no version\n",
            Stability::Snapshot,
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::BadRequest(_)),
        "expected BadRequest, got: {err:?}"
    );

    // AsyncAPI that is not even YAML.
    let err = spec_service::provide_spec(
        &repo,
        params(
            "svc",
            ApiType::AsyncApi,
            "asyncapi: '2.6.0'\nchannels: [oops",
            Stability::Snapshot,
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::BadRequest(_)),
        "expected BadRequest, got: {err:?}"
    );

    // Proto without the sanshain-version marker.
    let err = spec_service::provide_spec(
        &repo,
        params(
            "svc",
            ApiType::Proto,
            "syntax = \"proto3\";\nmessage M {}\n",
            Stability::Snapshot,
        ),
    )
    .await
    .unwrap_err();
    match err {
        AppError::BadRequest(msg) => assert!(
            msg.contains("sanshain-version"),
            "the proto refusal names the missing marker: {msg}"
        ),
        other => panic!("expected BadRequest, got: {other:?}"),
    }

    // Nothing was stored for any of the refusals.
    assert!(repo.list_producers().await.unwrap().is_empty());
}

#[tokio::test]
async fn asyncapi_channel_contract_ownership_on_ga_provides() {
    let repo = setup().await;

    // svc-a publishes the message first and becomes its owner.
    let owner_doc = asyncapi_with_id_type("1.0.0", "string");
    spec_service::provide_spec(
        &repo,
        params("svc-a", ApiType::AsyncApi, &owner_doc, Stability::Ga),
    )
    .await
    .unwrap();

    // A different Producer with a *different* payload schema is refused,
    // naming the owner.
    let conflicting = asyncapi_with_id_type("1.0.0", "integer");
    let err = spec_service::provide_spec(
        &repo,
        params("svc-b", ApiType::AsyncApi, &conflicting, Stability::Ga),
    )
    .await
    .unwrap_err();
    match err {
        AppError::Conflict(msg) => {
            assert!(
                msg.contains("owned by service 'svc-a'"),
                "the refusal names the owner: {msg}"
            );
            assert!(
                msg.contains("UserCreated") && msg.contains("user-created"),
                "the refusal names message and channel: {msg}"
            );
        }
        other => panic!("expected Conflict, got: {other:?}"),
    }

    // The same schema from another Producer is accepted (shared publisher).
    let identical = asyncapi_with_id_type("1.0.0", "string");
    spec_service::provide_spec(
        &repo,
        params("svc-b", ApiType::AsyncApi, &identical, Stability::Ga),
    )
    .await
    .unwrap();

    // The owner itself cannot retype the payload — even with a major bump the
    // channel truth must stay compatible.
    let retyped = asyncapi_with_id_type("2.0.0", "integer");
    let err = spec_service::provide_spec(
        &repo,
        params("svc-a", ApiType::AsyncApi, &retyped, Stability::Ga),
    )
    .await
    .unwrap_err();
    match err {
        AppError::BreakingChange(msg) => assert!(
            msg.contains("UserCreated"),
            "the refusal names the owned message: {msg}"
        ),
        other => panic!("expected BreakingChange, got: {other:?}"),
    }

    // A compatible widening by the owner goes through.
    let widened = r#"asyncapi: 2.6.0
info:
  title: T
  version: 2.0.0
channels:
  user-created:
    publish:
      message:
        name: UserCreated
        payload:
          type: object
          properties:
            id: { type: string }
            name: { type: string }
"#
    .to_string();
    spec_service::provide_spec(
        &repo,
        params("svc-a", ApiType::AsyncApi, &widened, Stability::Ga),
    )
    .await
    .unwrap();
    let contract = repo
        .get_channel_message_contract("user-created", "UserCreated")
        .await
        .unwrap()
        .expect("contract stored");
    assert!(
        contract.payload_yaml.contains("name"),
        "the stored contract carries the widened payload: {}",
        contract.payload_yaml
    );
}

#[tokio::test]
async fn endpoint_history_distinguishes_unknown_producer_from_no_versions() {
    let repo = setup().await;

    let err = spec_service::get_endpoint_history(&repo, "ghost", ApiType::OpenApi, "/x", "GET")
        .await
        .unwrap_err();
    match err {
        AppError::NotFound(msg) => assert_eq!(msg, "Producer not found"),
        other => panic!("expected NotFound, got: {other:?}"),
    }

    // A Producer that exists but has no version lines yet.
    repo.ensure_service("empty-svc").await.unwrap();
    let err = spec_service::get_endpoint_history(&repo, "empty-svc", ApiType::OpenApi, "/x", "GET")
        .await
        .unwrap_err();
    match err {
        AppError::NotFound(msg) => assert_eq!(msg, "No versions"),
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

#[tokio::test]
async fn diff_versions_refuses_an_unknown_producer() {
    let repo = setup().await;
    let err = spec_service::diff_versions(
        &repo,
        "ghost",
        ApiType::OpenApi,
        "1.0.0".parse().unwrap(),
        "1.1.0".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::NotFound(_)),
        "expected NotFound, got: {err:?}"
    );
}

// --- AsyncAPI subscribe harvesting (ai/improvements.md #6, ADR-0006) ---

#[cfg(test)]
fn asyncapi_subscribes_to(version: &str, extra_prop_line: &str) -> String {
    // A shipping service that subscribes to `user-created` (and, for the
    // resolve/unfulfilled test, an unowned channel). `extra_prop_line` lets a
    // test add a property the contract does not guarantee, to trigger drift.
    format!(
        r#"asyncapi: 2.6.0
info:
  title: shipping
  version: {version}
channels:
  user-created:
    subscribe:
      message:
        name: UserCreated
        payload:
          type: object
          properties:
            id: {{ type: string }}
{extra_prop_line}
  no-provider:
    subscribe:
      message:
        name: Ghost
        payload:
          type: object
"#
    )
}

#[tokio::test]
async fn asyncapi_subscribe_harvest_resolves_unfulfilled_and_retracts() {
    let repo = setup().await;
    repo.ensure_service("orders").await.unwrap();
    repo.ensure_service("shipping").await.unwrap();

    // orders publishes the contract at GA.
    spec_service::provide_spec(
        &repo,
        params(
            "orders",
            ApiType::AsyncApi,
            &asyncapi_with_id_type("1.0.0", "string"),
            Stability::Ga,
        ),
    )
    .await
    .unwrap();

    // shipping subscribes to it (a satisfiable subset) and to an unowned channel.
    let res = spec_service::provide_spec(
        &repo,
        params(
            "shipping",
            ApiType::AsyncApi,
            &asyncapi_subscribes_to("1.0.0", ""),
            Stability::Snapshot,
        ),
    )
    .await
    .unwrap();

    assert_eq!(res.harvested_subscriptions.len(), 2);
    let owned = res
        .harvested_subscriptions
        .iter()
        .find(|h| h.channel == "user-created")
        .unwrap();
    assert_eq!(owned.owner.as_deref(), Some("orders"));
    assert!(owned.drift.is_none());
    let unfulfilled = res
        .harvested_subscriptions
        .iter()
        .find(|h| h.channel == "no-provider")
        .unwrap();
    assert_eq!(unfulfilled.owner, None, "no GA provider owns this channel");

    let edges = repo.list_current_harvested_subscriptions().await.unwrap();
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().any(|e| e.client == "shipping"
        && e.channel == "user-created"
        && e.owner.as_deref() == Some("orders")));

    // A re-provide that drops the subscriptions retracts the edges.
    let empty = r#"asyncapi: 2.6.0
info:
  title: shipping
  version: 1.0.1
channels: {}
"#;
    spec_service::provide_spec(
        &repo,
        params("shipping", ApiType::AsyncApi, empty, Stability::Snapshot),
    )
    .await
    .unwrap();
    assert!(
        repo.list_current_harvested_subscriptions()
            .await
            .unwrap()
            .is_empty(),
        "dropped subscriptions are retracted"
    );
}

#[tokio::test]
async fn asyncapi_subscribe_drift_blocks_at_ga_and_is_advisory_on_snapshot() {
    let repo = setup().await;
    repo.ensure_service("orders").await.unwrap();
    repo.ensure_service("shipping").await.unwrap();
    spec_service::provide_spec(
        &repo,
        params(
            "orders",
            ApiType::AsyncApi,
            &asyncapi_with_id_type("1.0.0", "string"),
            Stability::Ga,
        ),
    )
    .await
    .unwrap();

    // The expectation reads `ssn`, which the contract does not guarantee.
    let drifting = asyncapi_subscribes_to("1.0.0", "            ssn: { type: string }");

    // On a snapshot the provide succeeds with an advisory drift note.
    let res = spec_service::provide_spec(
        &repo,
        params(
            "shipping",
            ApiType::AsyncApi,
            &drifting,
            Stability::Snapshot,
        ),
    )
    .await
    .unwrap();
    let owned = res
        .harvested_subscriptions
        .iter()
        .find(|h| h.channel == "user-created")
        .unwrap();
    assert!(
        owned.drift.as_deref().unwrap_or_default().contains("ssn"),
        "advisory drift names the property: {:?}",
        owned.drift
    );

    // On a GA provide the same drift is fatal (409).
    let err = spec_service::provide_spec(
        &repo,
        params("shipping", ApiType::AsyncApi, &drifting, Stability::Ga),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::Conflict(_)),
        "GA drift must be a 409, got: {err:?}"
    );
}

#[tokio::test]
async fn byte_identical_trunk_reprovide_moves_harvest_to_main() {
    let repo = setup().await;
    repo.ensure_service("orders").await.unwrap();
    repo.ensure_service("shipping").await.unwrap();
    spec_service::provide_spec(
        &repo,
        params(
            "orders",
            ApiType::AsyncApi,
            &asyncapi_with_id_type("1.0.0", "string"),
            Stability::Ga,
        ),
    )
    .await
    .unwrap();

    let sub = asyncapi_subscribes_to("1.0.0", "");
    // Snapshot provide (trunk = false): the harvest is a dev-view edge.
    spec_service::provide_spec(
        &repo,
        params("shipping", ApiType::AsyncApi, &sub, Stability::Snapshot),
    )
    .await
    .unwrap();
    let dev = repo.list_current_harvested_subscriptions().await.unwrap();
    assert!(dev.iter().all(|e| !e.trunk), "snapshot harvest is dev-only");

    // A byte-identical *trunk* re-provide is a content no-op, but the harvest
    // must still move into the main graph (the fast path used to skip it).
    let trunk_reprovide = spec_service::ProvideSpecParams {
        producername: "shipping",
        api_type: ApiType::AsyncApi,
        content: &sub,
        stability: Stability::Snapshot,
        dry_run: false,
        trunk: true,
        tag: None,
        caller: Some(sanshain_service::domain::permissions::Actor::test_caller()),
        require_prior_content_match: false,
    };
    let res = spec_service::provide_spec(&repo, trunk_reprovide)
        .await
        .unwrap();
    assert!(
        !res.harvested_subscriptions.is_empty(),
        "a no-op re-provide still reports its harvest"
    );
    let after = repo.list_current_harvested_subscriptions().await.unwrap();
    assert!(
        after.iter().any(|e| e.channel == "user-created" && e.trunk),
        "the trunk re-provide moved the harvest into the main graph"
    );
}
