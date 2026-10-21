//! One scenario for both engines: a session and an API token that expired a
//! minute ago — the same calendar day, which is where a text comparison of
//! `2026-10-21T10:00:00+00:00` against `2026-10-21 10:01:00` went wrong — are
//! refused by the lookups and removed by the hourly cleanup, while fresh ones
//! survive both.

use sanshain_service::domain::models::ApiTokenScope;
use sanshain_service::domain::ports::{NewApiToken, SpecRepository};

// `cfg(test)` marks the helper as test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
pub async fn expired_credentials_are_refused_and_cleaned(repo: &impl SpecRepository) {
    let user = repo.create_user("expiry-user", "hash", true).await.unwrap();
    let now = chrono::Utc::now();
    let expired = (now - chrono::Duration::seconds(60)).to_rfc3339();
    let fresh = (now + chrono::Duration::days(1)).to_rfc3339();

    repo.create_session_with_token(user.id, "expired-session", &expired)
        .await
        .unwrap();
    repo.create_session_with_token(user.id, "fresh-session", &fresh)
        .await
        .unwrap();
    for (id, hash, expires_at) in [
        ("t-expired", "hash-expired", expired.as_str()),
        ("t-fresh", "hash-fresh", fresh.as_str()),
    ] {
        repo.create_api_token(NewApiToken {
            id,
            user_id: user.id,
            name: id,
            token_hash: hash,
            scope: ApiTokenScope::Full,
            created_at: &now.to_rfc3339(),
            expires_at,
        })
        .await
        .unwrap();
    }

    assert!(
        repo.validate_session("expired-session")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo.validate_session("fresh-session")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repo.validate_api_token("hash-expired")
            .await
            .unwrap()
            .is_none(),
        "a token expired a minute ago must be refused, not valid until the end of the day"
    );
    assert!(
        repo.validate_api_token("hash-fresh")
            .await
            .unwrap()
            .is_some()
    );

    let cutoff = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    assert_eq!(repo.delete_expired_credentials(&cutoff).await.unwrap(), 2);
    assert!(
        repo.validate_session("fresh-session")
            .await
            .unwrap()
            .is_some()
    );
    let tokens = repo.list_api_tokens(user.id).await.unwrap();
    assert_eq!(
        tokens.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec!["t-fresh"]
    );
}
