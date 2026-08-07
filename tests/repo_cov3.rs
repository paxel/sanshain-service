//! Repository-seam coverage for the auth/roles/groups/maintainers/audit/
//! favorites query methods that the router suites do not reach. Every call
//! goes through `CachedSpecRepository` wrapping `DatabaseRepo::Sqlite`, so the
//! cache pass-throughs and the enum dispatcher are exercised together with the
//! SQLite implementation.

use sanshain_service::domain::models::GroupSource;
use sanshain_service::domain::ports::{NewAuditLog, SpecRepository};
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;

const PAST: &str = "2000-01-01T00:00:00Z";
const FUTURE: &str = "2100-01-01T00:00:00Z";

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn setup() -> CachedSpecRepository {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 64)
}

#[tokio::test]
async fn ping_and_service_name_lookup() {
    let repo = setup().await;
    repo.ping().await.unwrap();

    let sid = repo.ensure_service("svc-a").await.unwrap();
    assert_eq!(
        repo.get_service_name_by_id(sid).await.unwrap(),
        Some("svc-a".to_string())
    );
    assert_eq!(repo.get_service_name_by_id(999_999).await.unwrap(), None);
}

#[tokio::test]
async fn session_lifecycle_hashes_tokens_and_honours_expiry() {
    let repo = setup().await;
    let user = repo.create_user("alice", "hash-a", true).await.unwrap();
    assert_eq!(repo.user_count().await.unwrap(), 1);

    // A generated-token session validates and names its user.
    let session = repo.create_session(user.id, FUTURE).await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert_eq!(session.expires_at, FUTURE);
    let (found_user, found_session) = repo
        .validate_session(&session.token)
        .await
        .unwrap()
        .expect("fresh session validates");
    assert_eq!(found_user.username, "alice");
    assert_eq!(found_session.user_id, user.id);

    // Unknown tokens and expired sessions do not validate.
    assert!(
        repo.validate_session("no-such-token")
            .await
            .unwrap()
            .is_none()
    );
    let expired = repo.create_session(user.id, PAST).await.unwrap();
    assert!(
        repo.validate_session(&expired.token)
            .await
            .unwrap()
            .is_none()
    );

    // A fixed-token session validates by that token until logout.
    let fixed = repo
        .create_session_with_token(user.id, "fixed-token-1", FUTURE)
        .await
        .unwrap();
    assert_eq!(fixed.token, "fixed-token-1");
    assert!(
        repo.validate_session("fixed-token-1")
            .await
            .unwrap()
            .is_some()
    );
    repo.delete_session("fixed-token-1").await.unwrap();
    assert!(
        repo.validate_session("fixed-token-1")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn update_password_replaces_the_stored_hash() {
    let repo = setup().await;
    let user = repo.create_user("bob", "old-hash", true).await.unwrap();
    repo.update_password(user.id, "new-hash").await.unwrap();
    let found = repo.find_user("bob").await.unwrap().expect("bob exists");
    assert_eq!(found.password_hash, "new-hash");
}

#[tokio::test]
async fn api_token_repo_lifecycle_validates_and_deletes_by_owner() {
    let repo = setup().await;
    let owner = repo.create_user("owner", "h", true).await.unwrap();
    let other = repo.create_user("other", "h", true).await.unwrap();

    repo.create_api_token("tok-1", owner.id, "ci", "hash-1", PAST, FUTURE)
        .await
        .unwrap();
    repo.create_api_token("tok-2", owner.id, "old", "hash-2", PAST, PAST)
        .await
        .unwrap();

    let listed = repo.list_api_tokens(owner.id).await.unwrap();
    let mut names: Vec<&str> = listed.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["ci", "old"]);
    let ci = listed.iter().find(|t| t.id == "tok-1").unwrap();
    assert_eq!(ci.user_id, owner.id);
    assert_eq!(ci.last_used_at, None);

    // A live token resolves its user and stamps last_used_at; expired and
    // unknown hashes resolve nothing.
    let resolved = repo
        .validate_api_token("hash-1")
        .await
        .unwrap()
        .expect("live token validates");
    assert_eq!(resolved.username, "owner");
    let listed = repo.list_api_tokens(owner.id).await.unwrap();
    let ci = listed.iter().find(|t| t.id == "tok-1").unwrap();
    assert!(ci.last_used_at.is_some(), "validation stamps last_used_at");
    assert!(repo.validate_api_token("hash-2").await.unwrap().is_none());
    assert!(repo.validate_api_token("nope").await.unwrap().is_none());

    // Deletion is owner-scoped.
    assert!(!repo.delete_api_token("tok-1", other.id).await.unwrap());
    assert!(repo.delete_api_token("tok-1", owner.id).await.unwrap());
    assert!(!repo.delete_api_token("tok-1", owner.id).await.unwrap());
}

#[tokio::test]
async fn roles_groups_and_effective_stored_roles() {
    let repo = setup().await;
    let user = repo.create_user("carol", "h", true).await.unwrap();

    // Direct grants are idempotent and revocable exactly once.
    repo.grant_user_role(user.id, "admin").await.unwrap();
    repo.grant_user_role(user.id, "admin").await.unwrap();
    assert_eq!(repo.list_user_roles(user.id).await.unwrap(), vec!["admin"]);
    assert!(repo.revoke_user_role(user.id, "admin").await.unwrap());
    assert!(!repo.revoke_user_role(user.id, "admin").await.unwrap());

    // Creating the same group twice answers the same row.
    let group = repo
        .create_group("devs", GroupSource::Native)
        .await
        .unwrap();
    let again = repo
        .create_group("devs", GroupSource::Native)
        .await
        .unwrap();
    assert_eq!(group.id, again.id);

    // Renames report whether the group existed.
    assert!(repo.rename_group(group.id, "developers").await.unwrap());
    assert!(!repo.rename_group(999_999, "ghost").await.unwrap());
    let groups = repo.list_groups().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "developers");
    assert_eq!(groups[0].source, GroupSource::Native);

    // Group roles are replaced wholesale.
    repo.set_group_roles(group.id, &["auditor".to_string(), "operator".to_string()])
        .await
        .unwrap();
    let mut roles = repo.list_group_roles(group.id).await.unwrap();
    roles.sort_unstable();
    assert_eq!(roles, vec!["auditor", "operator"]);
    repo.set_group_roles(group.id, &["auditor".to_string()])
        .await
        .unwrap();
    assert_eq!(
        repo.list_group_roles(group.id).await.unwrap(),
        vec!["auditor"]
    );

    // Effective roles combine direct grants and stored group membership.
    repo.grant_user_role(user.id, "admin").await.unwrap();
    repo.add_group_member(group.id, user.id).await.unwrap();
    repo.add_group_member(group.id, user.id).await.unwrap();
    assert_eq!(
        repo.list_group_member_ids(group.id).await.unwrap(),
        vec![user.id]
    );
    let mut effective = repo.effective_stored_roles(user.id).await.unwrap();
    effective.sort_unstable();
    assert_eq!(effective, vec!["admin", "auditor"]);

    // Membership removal reports whether they were a member.
    assert!(repo.remove_group_member(group.id, user.id).await.unwrap());
    assert!(!repo.remove_group_member(group.id, user.id).await.unwrap());
    assert_eq!(
        repo.effective_stored_roles(user.id).await.unwrap(),
        vec!["admin"]
    );

    // Deleting the group removes it from the listing.
    assert!(repo.delete_group(group.id).await.unwrap());
    assert!(!repo.delete_group(group.id).await.unwrap());
    assert!(repo.list_groups().await.unwrap().is_empty());
}

#[tokio::test]
async fn maintainer_scope_direct_and_via_group() {
    let repo = setup().await;
    let user = repo.create_user("dave", "h", true).await.unwrap();
    let sid = repo.ensure_service("svc-m").await.unwrap();

    // Direct assignment, idempotent.
    repo.add_user_maintainer(sid, user.id).await.unwrap();
    repo.add_user_maintainer(sid, user.id).await.unwrap();
    assert_eq!(
        repo.list_user_maintainer_ids(sid).await.unwrap(),
        vec![user.id]
    );
    assert!(repo.maintains_producer(user.id, sid).await.unwrap());
    assert_eq!(
        repo.list_maintained_producers(user.id).await.unwrap(),
        vec!["svc-m"]
    );
    assert!(repo.remove_user_maintainer(sid, user.id).await.unwrap());
    assert!(!repo.remove_user_maintainer(sid, user.id).await.unwrap());
    assert!(!repo.maintains_producer(user.id, sid).await.unwrap());

    // Group-mediated maintainership.
    let group = repo.create_group("ops", GroupSource::Native).await.unwrap();
    repo.add_group_member(group.id, user.id).await.unwrap();
    repo.add_group_maintainer(sid, group.id).await.unwrap();
    assert_eq!(
        repo.list_group_maintainer_ids(sid).await.unwrap(),
        vec![group.id]
    );
    assert!(repo.maintains_producer(user.id, sid).await.unwrap());
    assert_eq!(
        repo.list_maintained_producers(user.id).await.unwrap(),
        vec!["svc-m"]
    );
    assert!(repo.remove_group_maintainer(sid, group.id).await.unwrap());
    assert!(!repo.remove_group_maintainer(sid, group.id).await.unwrap());
    assert!(!repo.maintains_producer(user.id, sid).await.unwrap());
}

#[cfg(test)]
fn log<'a>(
    action: &'a str,
    service: Option<&'a str>,
    version: Option<&'a str>,
    action_type: Option<&'a str>,
) -> NewAuditLog<'a> {
    NewAuditLog {
        action,
        details: "details",
        service,
        version,
        action_type,
        diff: None,
        stream: None,
    }
}

#[tokio::test]
async fn audit_log_filters_and_recent_listing() {
    let repo = setup().await;
    repo.insert_audit_log(
        "ana",
        log("PROVIDE_SPEC", Some("svc-a"), Some("1.0.0"), Some("WRITE")),
    )
    .await
    .unwrap();
    repo.insert_audit_log(
        "ben",
        log("REQUIRE_SPEC", Some("svc-b"), Some("2.1.0"), Some("READ")),
    )
    .await
    .unwrap();
    repo.insert_audit_log("cyd", log("CLEAR_CACHE", None, None, Some("ADMIN")))
        .await
        .unwrap();

    let filter = |action_type: Option<&str>,
                  service: Option<&str>,
                  version: Option<&str>,
                  from: Option<&str>,
                  to: Option<&str>| {
        sanshain_service::domain::models::AuditLogFilter {
            from_date: from.map(String::from),
            to_date: to.map(String::from),
            action_type: action_type.map(String::from),
            service_wildcard: service.map(String::from),
            version_wildcard: version.map(String::from),
            stream: None,
            limit: 10,
        }
    };

    // Unfiltered: everything, newest first.
    let all = repo
        .get_audit_logs(filter(None, None, None, None, None))
        .await
        .unwrap();
    let actions: Vec<&str> = all.iter().map(|l| l.action.as_str()).collect();
    assert_eq!(actions, vec!["CLEAR_CACHE", "REQUIRE_SPEC", "PROVIDE_SPEC"]);

    // Each filter narrows on its own column.
    let writes = repo
        .get_audit_logs(filter(Some("WRITE"), None, None, None, None))
        .await
        .unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].username, "ana");

    let svc_b = repo
        .get_audit_logs(filter(None, Some("svc-b%"), None, None, None))
        .await
        .unwrap();
    assert_eq!(svc_b.len(), 1);
    assert_eq!(svc_b[0].action, "REQUIRE_SPEC");

    let v2 = repo
        .get_audit_logs(filter(None, None, Some("2.%"), None, None))
        .await
        .unwrap();
    assert_eq!(v2.len(), 1);
    assert_eq!(v2[0].version.as_deref(), Some("2.1.0"));

    // Date bounds: everything is after 2000 and before 2100.
    let bounded = repo
        .get_audit_logs(filter(None, None, None, Some(PAST), Some(FUTURE)))
        .await
        .unwrap();
    assert_eq!(bounded.len(), 3);
    let none_yet = repo
        .get_audit_logs(filter(None, None, None, None, Some(PAST)))
        .await
        .unwrap();
    assert_eq!(none_yet.len(), 0);

    // The recent listing honours its limit, newest first.
    let recent = repo.get_recent_audit_logs(2).await.unwrap();
    let actions: Vec<&str> = recent.iter().map(|l| l.action.as_str()).collect();
    assert_eq!(actions, vec!["CLEAR_CACHE", "REQUIRE_SPEC"]);
}

#[tokio::test]
async fn settings_overwrite_and_favorites_roundtrip() {
    let repo = setup().await;
    assert_eq!(repo.get_setting("nothing").await.unwrap(), None);
    repo.set_setting("mode", "one").await.unwrap();
    repo.set_setting("mode", "two").await.unwrap();
    assert_eq!(
        repo.get_setting("mode").await.unwrap(),
        Some("two".to_string())
    );

    let user = repo.create_user("fay", "h", true).await.unwrap();
    assert!(
        repo.get_user_favorites(user.id, "service")
            .await
            .unwrap()
            .is_empty()
    );
    repo.add_user_favorite(user.id, "service", "svc-a")
        .await
        .unwrap();
    repo.add_user_favorite(user.id, "client", "web-ui")
        .await
        .unwrap();
    assert_eq!(
        repo.get_user_favorites(user.id, "service").await.unwrap(),
        vec!["svc-a"]
    );
    assert_eq!(
        repo.get_user_favorites(user.id, "client").await.unwrap(),
        vec!["web-ui"]
    );
    repo.remove_user_favorite(user.id, "service", "svc-a")
        .await
        .unwrap();
    assert!(
        repo.get_user_favorites(user.id, "service")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn service_tags_accumulate_per_service() {
    let repo = setup().await;
    let sid = repo.ensure_service("svc-t").await.unwrap();
    repo.add_service_tags(sid, &["billing".to_string(), "beta".to_string()])
        .await
        .unwrap();
    let tags = repo.get_all_service_tags().await.unwrap();
    let mut svc_tags = tags.get("svc-t").cloned().unwrap_or_default();
    svc_tags.sort_unstable();
    assert_eq!(svc_tags, vec!["beta", "billing"]);
}

#[tokio::test]
async fn clients_without_dependencies_are_not_listed_as_consumers() {
    let repo = setup().await;
    let cid = repo.ensure_client("web-ui").await.unwrap();
    let cid_again = repo.ensure_client("web-ui").await.unwrap();
    assert_eq!(cid, cid_again, "ensure_client is idempotent");
    // The consumers listing only names clients with recorded dependencies, so
    // a bare client row stays invisible.
    assert_eq!(repo.list_consumers().await.unwrap(), Vec::<String>::new());
    assert!(
        repo.list_consumer_endpoints("web-ui")
            .await
            .unwrap()
            .is_empty()
    );
}
