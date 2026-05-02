use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::application::spec_service;
use sanshain_service::domain::models::{ApiType, AppError};

fn openapi_ok() -> String {
    r#"
openapi: 3.0.0
info: { title: S, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  id: { type: integer }
"#
    .to_string()
}

fn openapi_breaking_remove_status() -> String {
    // Same path, but remove 200 response entirely (breaking on protected branches)
    r#"
openapi: 3.0.0
info: { title: S, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '201': { description: created }
"#
    .to_string()
}

fn openapi_compatible_additional_response() -> String {
    // Adds a new 201 response while keeping 200, which is backward-compatible.
    r#"
openapi: 3.0.0
info: { title: S, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  id: { type: integer }
        '201': { description: created }
"#
    .to_string()
}

#[tokio::test]
async fn protected_branch_rejects_breaking_change() {
    let repo = MockRepo::new(); // has "main" protected by default
    // Seed initial spec on protected branch
    let _ = spec_service::provide_spec(&repo, "svc", "main", ApiType::OpenApi, &openapi_ok(), None)
        .await
        .expect("seed ok");

    // Attempt a breaking update on protected branch
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        &openapi_breaking_remove_status(),
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));
}

#[tokio::test]
async fn unprotected_branch_allows_update_and_increments_version() {
    let repo = MockRepo::new();
    // Unprotected branch "dev" should allow the update
    let r1 = spec_service::provide_spec(&repo, "svc", "dev", ApiType::OpenApi, &openapi_ok(), None)
        .await
        .expect("first provide on dev");
    assert_eq!(r1.version, 1);

    // A second identical update should NOT bump version again (no-op)
    let r2 = spec_service::provide_spec(&repo, "svc", "dev", ApiType::OpenApi, &openapi_ok(), None)
        .await
        .expect("second provide on dev");
    assert_eq!(r2.version, r1.version);
}

#[tokio::test]
async fn dry_run_reports_inserts_then_noop() {
    let repo = MockRepo::new();
    // First time on a fresh branch should report inserts > 0
    let r1 =
        spec_service::provide_spec_dry_run(&repo, "svc", "draft", ApiType::OpenApi, &openapi_ok())
            .await
            .expect("dry run ok");
    assert!(r1.changes.inserts > 0);
    assert_eq!(r1.changes.updates, 0);
    assert_eq!(r1.changes.deletes, 0);

    // Actually apply once
    let _ =
        spec_service::provide_spec(&repo, "svc", "draft", ApiType::OpenApi, &openapi_ok(), None)
            .await
            .expect("apply ok");

    // Now dry-run again with same content should be a no-op
    let r2 =
        spec_service::provide_spec_dry_run(&repo, "svc", "draft", ApiType::OpenApi, &openapi_ok())
            .await
            .expect("dry run noop");
    assert_eq!(r2.changes.inserts, 0);
    assert_eq!(r2.changes.updates, 0);
    assert_eq!(r2.changes.deletes, 0);
}

#[tokio::test]
async fn protected_branch_allows_compatible_change() {
    let repo = MockRepo::new();
    let r1 =
        spec_service::provide_spec(&repo, "svc", "main", ApiType::OpenApi, &openapi_ok(), None)
            .await
            .expect("seed on main");
    assert_eq!(r1.version, 1);

    // Adding an extra response is backward-compatible, should be accepted on protected branch
    let r2 = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        &openapi_compatible_additional_response(),
        None,
    )
    .await
    .expect("compatible update on main");
    assert!(r2.version >= r1.version, "version should not decrease");
}

#[tokio::test]
async fn base_version_conflict_is_reported() {
    let repo = MockRepo::new();
    let r1 =
        spec_service::provide_spec(&repo, "svc", "dev2", ApiType::OpenApi, &openapi_ok(), None)
            .await
            .expect("seed dev2");
    assert_eq!(r1.version, 1);

    // Provide again but claim base_version 0 (outdated) — should be a conflict
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "dev2",
        ApiType::OpenApi,
        &openapi_ok(),
        Some(0),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));
}
