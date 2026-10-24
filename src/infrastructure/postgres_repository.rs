use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use tracing::instrument;

/// Row shape for `spec_versions` metadata queries (id, service_id, api_type,
/// major, minor, patch, stability, content_hash, provided_by, created_at,
/// updated_at, last_required_at, trunk_provided_at) — everything but the
/// document. `service_id` and the version components are `INTEGER` columns on
/// PostgreSQL: ids are BIGINT (`i64`), version parts INTEGER (`i32`).
type SpecVersionRow = (
    i64,
    i64,
    String,
    i32,
    i32,
    i32,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn spec_version_from_row(row: SpecVersionRow) -> Result<SpecVersionMeta, RepositoryError> {
    let (
        id,
        service_id,
        api_type,
        major,
        minor,
        patch,
        stability,
        content_hash,
        provided_by,
        created_at,
        updated_at,
        last_required_at,
        trunk_provided_at,
    ) = row;
    Ok(SpecVersionMeta {
        id,
        service_id,
        api_type: api_type
            .parse()
            .map_err(|e: String| RepositoryError::Internal(e))?,
        version: SemVer::new(major as u32, minor as u32, patch as u32),
        stability: stability
            .parse()
            .map_err(|e: String| RepositoryError::Internal(e))?,
        content_hash,
        provided_by,
        created_at,
        updated_at,
        last_required_at,
        trunk_provided_at,
    })
}

/// Row shape for audit-log queries (id, timestamp, username, action, details,
/// service, version, action_type, diff, stream).
type AuditLogRow = (
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Hashes a session token with SHA-256 so that only the hash is stored at rest.
/// The raw token is returned to the client; lookups hash the incoming token.
fn hash_session_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

use super::pin_rows::{
    PinRow, SystemContextRow, VersionOriginRow, links_json, periphery_track_key, pin_row_to_info,
    system_context_from_row, tags_json, tickets_json, version_origin_from_row,
};
use super::webhook_rows::{
    DeliveryRow, WebhookRow, data_from_json, data_json, delivery_columns, delivery_from_row,
    encode_event_types, webhook_columns, webhook_from_row,
};
use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewApiToken, NewAuditLog, NewWebhook, RecordDependencyParams,
    RecordTrunkPinParams, RepositoryError, RewriteSpecText, SpecRepository, UpsertSpecVersion,
};

struct ApiTokenRow {
    id: String,
    user_id: i64,
    name: String,
    token_hash: String,
    scope: String,
    created_at: String,
    expires_at: String,
    last_used_at: Option<String>,
    issued_by: Option<String>,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ApiTokenRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            name: row.try_get("name")?,
            token_hash: row.try_get("token_hash")?,
            scope: row.try_get("scope")?,
            created_at: row.try_get("created_at")?,
            expires_at: row.try_get("expires_at")?,
            last_used_at: row.try_get("last_used_at")?,
            issued_by: row.try_get("issued_by")?,
        })
    }
}

impl From<ApiTokenRow> for ApiToken {
    fn from(row: ApiTokenRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            name: row.name,
            token_hash: row.token_hash,
            // An unknown spelling narrows rather than widens: a row this code
            // cannot read is treated as read-only.
            scope: ApiTokenScope::parse(&row.scope).unwrap_or(ApiTokenScope::ReadOnly),
            created_at: row.created_at,
            expires_at: row.expires_at,
            last_used_at: row.last_used_at,
            issued_by: row.issued_by,
        }
    }
}

/// `user_profiles` as selected: user id, last sign-in, display name, avatar
/// hash, directory id, own avatar hash.
type UserProfileRow = (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn user_profile_from_row(
    (user_id, last_sign_in, display_name, avatar_hash, directory_id, own_avatar_hash): UserProfileRow,
) -> UserProfile {
    UserProfile {
        user_id,
        last_sign_in: last_sign_in.as_deref().and_then(SignInMethod::parse),
        display_name,
        avatar_hash,
        own_avatar_hash,
        directory_id,
    }
}

#[derive(Clone)]

pub struct PostgresSpecRepository {
    pub pool: PgPool,
}

impl PostgresSpecRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), sqlx::migrate::MigrateError> {
        tracing::info!("Running PostgreSQL migrations...");
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(
            "src/infrastructure/migrations/postgres",
        ))
        .await?;
        migrator.run(&self.pool).await?;
        Ok(())
    }
}

impl SpecRepository for PostgresSpecRepository {
    async fn ping(&self) -> Result<(), RepositoryError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    #[instrument(skip_all)]
    async fn upsert_spec_version(
        &self,
        params: UpsertSpecVersion<'_>,
    ) -> Result<i64, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Insert-or-overwrite keeps the row's identity and `created_at` on an
        // overwrite/promotion; only the content, stability, attribution and
        // `updated_at` move.
        //
        // A compare-and-set caller never inserts: the row it read must still
        // exist and carry the expected hash, so it gets a plain conditional
        // UPDATE. The UPDATE re-checks its predicate on the current row version
        // after locking it, so a concurrently-committed delete or release makes
        // it match zero rows — an INSERT-arm EXISTS guard would evaluate on the
        // statement snapshot while ON CONFLICT arbitrates on the live index,
        // letting a racing DELETE resurrect the version as a fresh GA row.
        let upsert = if let Some(expected) = params.expected_prior_hash {
            sqlx::query(
                r#"
                UPDATE spec_versions SET
                    stability = $6,
                    content = $7,
                    content_hash = $8,
                    provided_by = $9,
                    updated_at = $10
                 WHERE service_id = $1 AND api_type = $2 AND major = $3 AND minor = $4 AND patch = $5
                   AND stability != 'ga'
                   AND content_hash = $11
                "#,
            )
            .bind(params.service_id)
            .bind(params.api_type.as_str())
            .bind(params.version.major as i32)
            .bind(params.version.minor as i32)
            .bind(params.version.patch as i32)
            .bind(params.stability.as_str())
            .bind(params.content)
            .bind(params.content_hash)
            .bind(params.provided_by)
            .bind(params.now_iso)
            .bind(expected)
        } else {
            sqlx::query(
                r#"
                INSERT INTO spec_versions
                  (service_id, api_type, major, minor, patch, stability, content, content_hash, provided_by, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT(service_id, api_type, major, minor, patch) DO UPDATE SET
                    stability = EXCLUDED.stability,
                    content = EXCLUDED.content,
                    content_hash = EXCLUDED.content_hash,
                    provided_by = EXCLUDED.provided_by,
                    updated_at = EXCLUDED.updated_at
                WHERE spec_versions.stability != 'ga'
                "#,
            )
            .bind(params.service_id)
            .bind(params.api_type.as_str())
            .bind(params.version.major as i32)
            .bind(params.version.minor as i32)
            .bind(params.version.patch as i32)
            .bind(params.stability.as_str())
            .bind(params.content)
            .bind(params.content_hash)
            .bind(params.provided_by)
            .bind(params.now_iso)
            .bind(params.now_iso)
        }
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Zero rows written means the guard refused — the last line of defense
        // against racing writers: for a CAS caller the row vanished, was
        // released, or no longer carries the expected hash; for a plain provide
        // a GA row is immutable even against a racing writer.
        if upsert.rows_affected() == 0 {
            return Err(RepositoryError::Conflict);
        }

        let (id,): (i64,) = sqlx::query_as(
            "SELECT id FROM spec_versions WHERE service_id = $1 AND api_type = $2 AND major = $3 AND minor = $4 AND patch = $5",
        )
        .bind(params.service_id)
        .bind(params.api_type.as_str())
        .bind(params.version.major as i32)
        .bind(params.version.minor as i32)
        .bind(params.version.patch as i32)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // A version's endpoint set is complete by definition — replace it
        // wholesale rather than diffing in SQL.
        sqlx::query("DELETE FROM endpoints WHERE spec_version_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for endpoint in &params.endpoints {
            sqlx::query(
                "INSERT INTO endpoints (spec_version_id, api_type, path, normalized_path, method, yaml_content, deprecated) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(id)
            .bind(endpoint.api_type.as_str())
            .bind(&endpoint.path)
            .bind(&endpoint.normalized_path)
            .bind(&endpoint.method)
            .bind(&endpoint.yaml_content)
            .bind(endpoint.deprecated)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        write_outbox(&mut tx, &params.outbox).await?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(id)
    }

    #[instrument(skip_all)]
    async fn rewrite_spec_text(&self, params: RewriteSpecText<'_>) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("UPDATE spec_versions SET content = $1, content_hash = $2 WHERE id = $3")
            .bind(params.content)
            .bind(params.content_hash)
            .bind(params.spec_version_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for (endpoint_id, text) in params.endpoint_texts {
            sqlx::query(
                "UPDATE endpoints SET yaml_content = $1 WHERE id = $2 AND spec_version_id = $3",
            )
            .bind(text)
            .bind(endpoint_id)
            .bind(params.spec_version_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn find_spec_version(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Option<SpecVersionMeta>, RepositoryError> {
        let row: Option<SpecVersionRow> = sqlx::query_as(
            "SELECT id, service_id, api_type, major, minor, patch, stability, content_hash, provided_by, created_at, updated_at, last_required_at, trunk_provided_at \
             FROM spec_versions WHERE service_id = $1 AND api_type = $2 AND major = $3 AND minor = $4 AND patch = $5",
        )
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major as i32)
        .bind(version.minor as i32)
        .bind(version.patch as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        row.map(spec_version_from_row).transpose()
    }

    async fn list_spec_versions(
        &self,
        service_id: i64,
    ) -> Result<Vec<SpecVersionMeta>, RepositoryError> {
        let rows: Vec<SpecVersionRow> = sqlx::query_as(
            "SELECT id, service_id, api_type, major, minor, patch, stability, content_hash, provided_by, created_at, updated_at, last_required_at, trunk_provided_at \
             FROM spec_versions WHERE service_id = $1 ORDER BY api_type, major, minor, patch",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(spec_version_from_row).collect()
    }

    async fn list_all_spec_versions(
        &self,
    ) -> Result<Vec<(String, SpecVersionMeta, i64)>, RepositoryError> {
        type Row = (
            String,
            i64,
            i64,
            String,
            i32,
            i32,
            i32,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
        );
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT s.name, v.id, v.service_id, v.api_type, v.major, v.minor, v.patch, v.stability,
                   v.content_hash, v.provided_by, v.created_at, v.updated_at, v.last_required_at,
                   v.trunk_provided_at,
                   (SELECT COUNT(*) FROM endpoints e WHERE e.spec_version_id = v.id) AS endpoint_count
            FROM spec_versions v
            JOIN services s ON s.id = v.service_id
            ORDER BY s.name, v.api_type, v.major, v.minor, v.patch
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                let (
                    name,
                    id,
                    service_id,
                    api_type,
                    major,
                    minor,
                    patch,
                    stability,
                    content_hash,
                    provided_by,
                    created_at,
                    updated_at,
                    last_required_at,
                    trunk_provided_at,
                    endpoint_count,
                ) = row;
                let meta = spec_version_from_row((
                    id,
                    service_id,
                    api_type,
                    major,
                    minor,
                    patch,
                    stability,
                    content_hash,
                    provided_by,
                    created_at,
                    updated_at,
                    last_required_at,
                    trunk_provided_at,
                ))?;
                Ok((name, meta, endpoint_count))
            })
            .collect()
    }

    async fn get_spec_content(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT content FROM spec_versions WHERE id = $1")
                .bind(spec_version_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }

    async fn delete_spec_version(&self, spec_version_id: i64) -> Result<bool, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_version_origin WHERE spec_version_id = $1")
            .bind(spec_version_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // A deleted version is no longer what trunk carries; its closed member
        // record stays as history.
        sqlx::query(
            "UPDATE trunk_member_versions AS m SET valid_to = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             WHERE m.valid_to IS NULL AND EXISTS (SELECT 1 FROM spec_versions sv WHERE sv.id = $1 \
             AND sv.service_id = m.service_id AND sv.api_type = m.api_type \
             AND sv.major = m.major AND sv.minor = m.minor AND sv.patch = m.patch)",
        )
        .bind(spec_version_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM spec_versions WHERE id = $1")
            .bind(spec_version_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn touch_spec_version_required(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE spec_versions SET last_required_at = $1 WHERE id = $2")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn touch_spec_version_provided(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE spec_versions SET updated_at = $1 WHERE id = $2")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn set_version_origin(
        &self,
        spec_version_id: i64,
        origin: &VersionOrigin,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        if origin.is_empty() {
            sqlx::query("DELETE FROM spec_version_origin WHERE spec_version_id = $1")
                .bind(spec_version_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO spec_version_origin \
                   (spec_version_id, build_url, change_url, tickets_json, note, trunk_build, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (spec_version_id) DO UPDATE SET \
                   build_url = EXCLUDED.build_url, change_url = EXCLUDED.change_url, \
                   tickets_json = EXCLUDED.tickets_json, note = EXCLUDED.note, \
                   trunk_build = EXCLUDED.trunk_build, updated_at = EXCLUDED.updated_at",
        )
        .bind(spec_version_id)
        .bind(origin.build_url.as_deref())
        .bind(origin.change_url.as_deref())
        .bind(tickets_json(origin)?)
        .bind(origin.note.as_deref())
        .bind(origin.trunk_build)
        .bind(now_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_version_origin(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<VersionOrigin>, RepositoryError> {
        let row: Option<VersionOriginRow> = sqlx::query_as(
            "SELECT spec_version_id, build_url, change_url, tickets_json, note, trunk_build \
             FROM spec_version_origin WHERE spec_version_id = $1",
        )
        .bind(spec_version_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        row.map(|row| version_origin_from_row(row).map(|(_, o)| o))
            .transpose()
    }

    async fn list_version_origins(&self) -> Result<BTreeMap<i64, VersionOrigin>, RepositoryError> {
        let rows: Vec<VersionOriginRow> = sqlx::query_as(
            "SELECT spec_version_id, build_url, change_url, tickets_json, note, trunk_build \
             FROM spec_version_origin",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(version_origin_from_row).collect()
    }

    async fn touch_spec_version_trunk(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        // Marker and member record are one act: a failure between them would
        // leave the main view's live and historical answers disagreeing.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("UPDATE spec_versions SET trunk_provided_at = $1 WHERE id = $2")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // The open record of this entry's line, compared with this entry.
        macro_rules! line_sql {
            () => {
                "m.valid_to IS NULL AND EXISTS (SELECT 1 FROM spec_versions sv WHERE sv.id = $2 \
                    AND sv.service_id = m.service_id AND sv.api_type = m.api_type"
            };
        }
        macro_rules! same_sql {
            () => {
                "AND sv.major = m.major AND sv.minor = m.minor AND sv.patch = m.patch)"
            };
        }
        sqlx::query(concat!(
            "UPDATE trunk_member_versions AS m SET valid_to = $1 WHERE ",
            line_sql!(),
            " AND NOT (sv.major = m.major AND sv.minor = m.minor AND sv.patch = m.patch))"
        ))
        .bind(now_iso)
        .bind(spec_version_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let refreshed = sqlx::query(concat!(
            "UPDATE trunk_member_versions AS m SET last_provided_at = $1 WHERE ",
            line_sql!(),
            " ",
            same_sql!()
        ))
        .bind(now_iso)
        .bind(spec_version_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        if refreshed.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO trunk_member_versions (service_id, service_name, api_type, major, minor, patch, valid_from, last_provided_at) \
                 SELECT sv.service_id, s.name, sv.api_type, sv.major, sv.minor, sv.patch, $1, $1 \
                 FROM spec_versions sv JOIN services s ON s.id = sv.service_id WHERE sv.id = $2",
            )
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn insert_branch(
        &self,
        name: &str,
        created_at: &str,
        created_by: &str,
        source: &str,
        as_of: &str,
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<i64, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let res: Result<(i64,), sqlx::Error> = sqlx::query_as(
            "INSERT INTO sanshain_branches (name, created_at, created_by, source, as_of) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(name)
        .bind(created_at)
        .bind(created_by)
        .bind(source)
        .bind(as_of)
        .fetch_one(&mut *tx)
        .await;
        let id = match res {
            Ok((id,)) => id,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                return Err(RepositoryError::Conflict);
            }
            Err(e) => return Err(RepositoryError::Internal(e.to_string())),
        };
        write_outbox(&mut tx, &outbox).await?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(id)
    }

    async fn find_branch(&self, name: &str) -> Result<Option<BranchInfo>, RepositoryError> {
        let row: Option<(i64, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, name, created_at, created_by, source, as_of FROM sanshain_branches WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(
            |(id, name, created_at, created_by, source, as_of)| BranchInfo {
                id,
                name,
                created_at,
                created_by,
                source,
                as_of,
            },
        ))
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, RepositoryError> {
        let rows: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, name, created_at, created_by, source, as_of FROM sanshain_branches ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(
                |(id, name, created_at, created_by, source, as_of)| BranchInfo {
                    id,
                    name,
                    created_at,
                    created_by,
                    source,
                    as_of,
                },
            )
            .collect())
    }

    async fn copy_trunk_graph_to_branch(
        &self,
        branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO branch_dependencies (branch_id, client_id, client_name, service_id, service_name, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
             SELECT $1, client_id, client_name, service_id, service_name, api_type, path, normalized_path, method, major, minor, patch, $2, $3 \
             FROM trunk_dependencies WHERE valid_from <= $4 AND (valid_to IS NULL OR valid_to > $4)",
        )
        .bind(branch_id)
        .bind(now_iso)
        .bind(now_iso)
        .bind(as_of)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // The periphery (#46) is part of the cut too.
        sqlx::query(
            "INSERT INTO peripheral_dependencies (track, branch_id, system_name, resource_ref, valid_from) \
             SELECT 'branch', $1, system_name, resource_ref, $2 FROM peripheral_dependencies \
             WHERE track = 'trunk' AND branch_id IS NULL AND valid_from <= $3 AND (valid_to IS NULL OR valid_to > $3)",
        )
        .bind(branch_id)
        .bind(now_iso)
        .bind(as_of)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn copy_branch_graph_to_branch(
        &self,
        target_branch_id: i64,
        source_branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO branch_dependencies (branch_id, client_id, client_name, service_id, service_name, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
             SELECT $1, client_id, client_name, service_id, service_name, api_type, path, normalized_path, method, major, minor, patch, $2, $3 \
             FROM branch_dependencies WHERE branch_id = $4 AND valid_from <= $5 AND (valid_to IS NULL OR valid_to > $5)",
        )
        .bind(target_branch_id)
        .bind(now_iso)
        .bind(now_iso)
        .bind(source_branch_id)
        .bind(as_of)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // The periphery (#46) is part of the cut too.
        sqlx::query(
            "INSERT INTO peripheral_dependencies (track, branch_id, system_name, resource_ref, valid_from) \
             SELECT 'branch', $1, system_name, resource_ref, $2 FROM peripheral_dependencies \
             WHERE track = 'branch' AND branch_id = $3 AND valid_from <= $4 AND (valid_to IS NULL OR valid_to > $4)",
        )
        .bind(target_branch_id)
        .bind(now_iso)
        .bind(source_branch_id)
        .bind(as_of)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_branch_pins(
        &self,
        branch_id: i64,
        at: Option<&str>,
    ) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        macro_rules! base_sql {
            () => {
                "SELECT t.client_name, t.service_name, t.api_type, t.major, t.minor, t.patch, t.path, t.normalized_path, t.method, t.valid_from, t.last_required_at \
             FROM branch_dependencies t \
             WHERE t.branch_id = $1"
            };
        }
        let rows: Vec<PinRow<i32>> = match at {
            Some(at) => {
                sqlx::query_as(concat!(base_sql!(), " AND t.valid_from <= $2 AND (t.valid_to IS NULL OR t.valid_to > $2) ORDER BY t.client_name, t.service_name, t.path, t.method"))
                .bind(branch_id)
                .bind(at)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as(concat!(base_sql!(), " AND t.valid_to IS NULL ORDER BY t.client_name, t.service_name, t.path, t.method"))
                .bind(branch_id)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(pin_row_to_info).collect()
    }

    async fn list_trunk_pins_at(&self, at: &str) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let rows: Vec<PinRow<i32>> = sqlx::query_as(
            "SELECT t.client_name, t.service_name, t.api_type, t.major, t.minor, t.patch, t.path, t.normalized_path, t.method, t.valid_from, t.last_required_at \
             FROM trunk_dependencies t \
             WHERE t.valid_from <= $1 AND (t.valid_to IS NULL OR t.valid_to > $1) \
             ORDER BY t.client_name, t.service_name, t.path, t.method",
        )
        .bind(at)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(pin_row_to_info).collect()
    }

    async fn list_graph_change_dates(
        &self,
        branch_id: Option<i64>,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = match branch_id {
            None => {
                sqlx::query_as(
                    "SELECT DISTINCT d FROM (SELECT valid_from AS d FROM trunk_dependencies \
                       UNION SELECT valid_to AS d FROM trunk_dependencies WHERE valid_to IS NOT NULL \
                       UNION SELECT valid_from AS d FROM peripheral_dependencies WHERE track = 'trunk' \
                       UNION SELECT valid_to AS d FROM peripheral_dependencies WHERE track = 'trunk' AND valid_to IS NOT NULL) x ORDER BY d",
                )
                .fetch_all(&self.pool)
                .await
            }
            Some(bid) => {
                sqlx::query_as(
                    // Member versions count as graph changes too: a hotfix
                    // tag-provided with no accompanying tagged require moves
                    // the branch without touching branch_dependencies, and the
                    // slider must still offer that instant.
                    "SELECT DISTINCT d FROM (\
                       SELECT valid_from AS d FROM branch_dependencies WHERE branch_id = $1 \
                       UNION SELECT valid_to AS d FROM branch_dependencies WHERE branch_id = $1 AND valid_to IS NOT NULL \
                       UNION SELECT valid_from AS d FROM branch_member_versions WHERE branch_id = $1 \
                       UNION SELECT valid_to AS d FROM branch_member_versions WHERE branch_id = $1 AND valid_to IS NOT NULL \
                       UNION SELECT valid_from AS d FROM peripheral_dependencies WHERE track = 'branch' AND branch_id = $1 \
                       UNION SELECT valid_to AS d FROM peripheral_dependencies WHERE track = 'branch' AND branch_id = $1 AND valid_to IS NOT NULL) x ORDER BY d",
                )
                .bind(bid)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|(d,)| d).collect())
    }

    async fn list_branch_memberships_for_service(
        &self,
        service_id: i64,
    ) -> Result<Vec<BranchMembership>, RepositoryError> {
        type Row = (String, i32, i32, i32, String);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT api_type, major, minor, patch, name FROM ( \
                 SELECT d.api_type, d.major, d.minor, d.patch, b.name \
                 FROM branch_dependencies d JOIN sanshain_branches b ON b.id = d.branch_id \
                 WHERE d.service_id = $1 AND d.valid_to IS NULL \
                 UNION \
                 SELECT m.api_type, m.major, m.minor, m.patch, b.name \
                 FROM branch_member_versions m JOIN sanshain_branches b ON b.id = m.branch_id \
                 WHERE m.service_id = $1 AND m.valid_to IS NULL \
             ) x ORDER BY name, api_type, major, minor, patch",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(|(api_type, major, minor, patch, branch)| {
                Ok(BranchMembership {
                    api_type: api_type
                        .parse()
                        .map_err(|e: String| RepositoryError::Internal(e))?,
                    version: SemVer::new(major as u32, minor as u32, patch as u32),
                    branch,
                })
            })
            .collect()
    }

    async fn list_branches_referencing(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT b.name FROM sanshain_branches b \
             WHERE EXISTS (SELECT 1 FROM branch_dependencies d WHERE d.branch_id = b.id AND d.valid_to IS NULL \
                           AND d.service_id = $1 AND d.api_type = $2 AND d.major = $3 AND d.minor = $4 AND d.patch = $5) \
                OR EXISTS (SELECT 1 FROM branch_member_versions m WHERE m.branch_id = b.id AND m.valid_to IS NULL \
                           AND m.service_id = $1 AND m.api_type = $2 AND m.major = $3 AND m.minor = $4 AND m.patch = $5) \
             ORDER BY b.name",
        )
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major as i32)
        .bind(version.minor as i32)
        .bind(version.patch as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    async fn close_expired_trunk_data(
        &self,
        cutoff_iso: &str,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        let closed = sqlx::query(
            "UPDATE trunk_dependencies SET valid_to = $1 WHERE valid_to IS NULL AND last_required_at < $2",
        )
        .bind(now_iso)
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE spec_versions SET trunk_provided_at = NULL WHERE trunk_provided_at IS NOT NULL AND trunk_provided_at < $1",
        )
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE trunk_member_versions SET valid_to = $1 WHERE valid_to IS NULL AND last_provided_at < $2",
        )
        .bind(now_iso)
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(closed.rows_affected())
    }

    async fn rename_branch(&self, branch_id: i64, new_name: &str) -> Result<(), RepositoryError> {
        let res = sqlx::query("UPDATE sanshain_branches SET name = $1 WHERE id = $2")
            .bind(new_name)
            .bind(branch_id)
            .execute(&self.pool)
            .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(RepositoryError::Conflict)
            }
            Err(e) => Err(RepositoryError::Internal(e.to_string())),
        }
    }

    async fn delete_branch(&self, branch_id: i64) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM sanshain_branches WHERE id = $1")
            .bind(branch_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_branch_pins(
        &self,
        branch_id: i64,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for p in pins {
            macro_rules! key_sql {
                () => {
                    "branch_id = $2 AND client_id = $3 AND service_id = $4 AND api_type = $5 AND normalized_path = $6 AND method = $7 AND valid_to IS NULL"
                };
            }
            sqlx::query(concat!(
                "UPDATE branch_dependencies SET valid_to = $1 WHERE ",
                key_sql!(),
                " AND NOT (major = $8 AND minor = $9 AND patch = $10)"
            ))
            .bind(p.now_iso)
            .bind(branch_id)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major as i32)
            .bind(p.version.minor as i32)
            .bind(p.version.patch as i32)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            let refreshed = sqlx::query(concat!(
                "UPDATE branch_dependencies SET last_required_at = $1 WHERE ",
                key_sql!(),
                " AND major = $8 AND minor = $9 AND patch = $10"
            ))
            .bind(p.now_iso)
            .bind(branch_id)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major as i32)
            .bind(p.version.minor as i32)
            .bind(p.version.patch as i32)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            if refreshed.rows_affected() == 0 {
                sqlx::query(
                    // See record_trunk_pins: the unique partial index settles
                    // the concurrent first-insert race.
                    // client_name/service_name denormalized by value (#26B).
                    "INSERT INTO branch_dependencies (branch_id, client_id, client_name, service_id, service_name, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
                     VALUES ($1, $2, (SELECT name FROM clients WHERE id = $2), $3, (SELECT name FROM services WHERE id = $3), $4, $5, $6, $7, $8, $9, $10, $11, $12) \
                     ON CONFLICT (branch_id, client_id, service_id, api_type, normalized_path, method) \
                     WHERE valid_to IS NULL \
                     DO UPDATE SET last_required_at = EXCLUDED.last_required_at",
                )
                .bind(branch_id)
                .bind(p.client_id)
                .bind(p.service_id)
                .bind(p.api_type.as_str())
                .bind(p.path)
                .bind(p.normalized_path)
                .bind(p.method)
                .bind(p.version.major as i32)
                .bind(p.version.minor as i32)
                .bind(p.version.patch as i32)
                .bind(p.now_iso)
                .bind(p.now_iso)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_branch_member_version(
        &self,
        branch_id: i64,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        // Close-then-insert is one act: a failure between the two would leave
        // the branch with no open member version at all, hiding the producer
        // from the membership lookups until a later tagged provide repairs it.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE branch_member_versions SET valid_to = $1 WHERE branch_id = $2 AND service_id = $3 AND api_type = $4 AND valid_to IS NULL AND NOT (major = $5 AND minor = $6 AND patch = $7)",
        )
        .bind(now_iso)
        .bind(branch_id)
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major as i32)
        .bind(version.minor as i32)
        .bind(version.patch as i32)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO branch_member_versions (branch_id, service_id, service_name, api_type, major, minor, patch, valid_from) \
             SELECT $1, $2, (SELECT name FROM services WHERE id = $2), $3, $4, $5, $6, $7 \
             WHERE NOT EXISTS (SELECT 1 FROM branch_member_versions WHERE branch_id = $1 AND service_id = $2 AND api_type = $3 AND valid_to IS NULL AND major = $4 AND minor = $5 AND patch = $6)",
        )
        .bind(branch_id)
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major as i32)
        .bind(version.minor as i32)
        .bind(version.patch as i32)
        .bind(now_iso)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_trunk_pins(
        &self,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for p in pins {
            macro_rules! key_sql {
                () => {
                    "client_id = $2 AND service_id = $3 AND api_type = $4 AND normalized_path = $5 AND method = $6 AND valid_to IS NULL"
                };
            }
            // Append semantics: a different version closes the open record…
            sqlx::query(concat!(
                "UPDATE trunk_dependencies SET valid_to = $1 WHERE ",
                key_sql!(),
                " AND NOT (major = $7 AND minor = $8 AND patch = $9)"
            ))
            .bind(p.now_iso)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major as i32)
            .bind(p.version.minor as i32)
            .bind(p.version.patch as i32)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // …the same version only refreshes the open record…
            let refreshed = sqlx::query(concat!(
                "UPDATE trunk_dependencies SET last_required_at = $1 WHERE ",
                key_sql!(),
                " AND major = $7 AND minor = $8 AND patch = $9"
            ))
            .bind(p.now_iso)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major as i32)
            .bind(p.version.minor as i32)
            .bind(p.version.patch as i32)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // …and a new pin key or a closed record gets a fresh open one.
            if refreshed.rows_affected() == 0 {
                // The unique partial index decides the race: a concurrent
                // transaction that inserted this key first wins, and this one
                // refreshes its row instead of erroring or duplicating it.
                sqlx::query(
                    // client_name/service_name denormalized by value (#26B); a
                    // reused placeholder fills each from its id, no extra bind.
                    "INSERT INTO trunk_dependencies (client_id, client_name, service_id, service_name, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
                     VALUES ($1, (SELECT name FROM clients WHERE id = $1), $2, (SELECT name FROM services WHERE id = $2), $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                     ON CONFLICT (client_id, service_id, api_type, normalized_path, method) \
                     WHERE valid_to IS NULL \
                     DO UPDATE SET last_required_at = EXCLUDED.last_required_at",
                )
                .bind(p.client_id)
                .bind(p.service_id)
                .bind(p.api_type.as_str())
                .bind(p.path)
                .bind(p.normalized_path)
                .bind(p.method)
                .bind(p.version.major as i32)
                .bind(p.version.minor as i32)
                .bind(p.version.patch as i32)
                .bind(p.now_iso)
                .bind(p.now_iso)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_current_trunk_pins(&self) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let rows: Vec<PinRow<i32>> = sqlx::query_as(
            "SELECT t.client_name, t.service_name, t.api_type, t.major, t.minor, t.patch, t.path, t.normalized_path, t.method, t.valid_from, t.last_required_at \
             FROM trunk_dependencies t \
             WHERE t.valid_to IS NULL \
             ORDER BY t.client_name, t.service_name, t.path, t.method",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(pin_row_to_info).collect()
    }

    async fn delete_expired_snapshots(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        // Use-based: a snapshot survives if it was provided (updated_at) OR
        // required (last_required_at) since the cutoff. GA never expires.
        let result = sqlx::query(
            r#"
            DELETE FROM spec_versions
            WHERE stability = 'snapshot'
              AND updated_at < $1
              AND (last_required_at IS NULL OR last_required_at < $2)
            "#,
        )
        .bind(cutoff_iso)
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Origins live apart from their versions (no foreign key): sweep the
        // ones whose version just went.
        sqlx::query(
            "DELETE FROM spec_version_origin WHERE spec_version_id NOT IN (SELECT id FROM spec_versions)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn list_version_dependents(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT c.name
            FROM dependencies d
            JOIN clients c ON c.id = d.client_id
            WHERE d.spec_version_id = $1
            ORDER BY c.name
            "#,
        )
        .bind(spec_version_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }
    async fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT name FROM services WHERE id = $1")
            .bind(service_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }
    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        sqlx::query("INSERT INTO services (name) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM services WHERE name = $1")
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn get_endpoints_for_version(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        type EndpointRow = (i64, String, String, String, String, String, bool);
        let rows: Vec<EndpointRow> = sqlx::query_as(
            "SELECT e.id, e.api_type, e.path, e.normalized_path, e.method, e.yaml_content, \
             e.deprecated \
             FROM endpoints e \
             WHERE e.spec_version_id = $1",
        )
        .bind(spec_version_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(id, api_type, path, normalized_path, method, yaml_content, deprecated)| {
                    EndpointRecord {
                        id: Some(id),
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        path,
                        normalized_path,
                        method,
                        yaml_content,
                        deprecated,
                    }
                },
            )
            .collect())
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        sqlx::query("INSERT INTO clients (name) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM clients WHERE name = $1")
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn find_endpoint(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool)>, RepositoryError> {
        let normalized_path = crate::openapi::lookup_path(api_type, path);
        let row: Option<(i64, String, bool)> = sqlx::query_as(
            r#"
            SELECT e.id, e.yaml_content, e.deprecated
            FROM endpoints e
            WHERE e.spec_version_id = $1 AND e.api_type = $2 AND e.normalized_path = $3 AND e.method = $4
            "#,
        )
        .bind(spec_version_id)
        .bind(api_type.as_str())
        .bind(normalized_path)
        .bind(method)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row)
    }

    #[instrument(skip_all)]
    async fn find_endpoints_bulk(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        let mut result = EndpointMap::new();
        if endpoints.is_empty() {
            return Ok(result);
        }

        // The wanted (normalized path, method) pairs travel as two array binds,
        // so the statement stays fixed whatever the number of endpoints.
        let (paths, methods): (Vec<String>, Vec<&str>) = endpoints
            .iter()
            .map(|(path, method)| (crate::openapi::lookup_path(api_type, path), method.as_str()))
            .unzip();

        type BulkRow = (i64, String, String, String, String, bool);
        let rows: Vec<BulkRow> = sqlx::query_as(
            "SELECT e.id, e.path, e.normalized_path, e.method, e.yaml_content, e.deprecated \
             FROM endpoints e \
             WHERE e.spec_version_id = $1 AND e.api_type = $2 \
             AND (e.normalized_path, e.method) IN (SELECT * FROM UNNEST($3::TEXT[], $4::TEXT[]))",
        )
        .bind(spec_version_id)
        .bind(api_type.as_str())
        .bind(paths)
        .bind(methods)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Key the result by what the caller *asked for*, matched via the
        // normalized path, so lenient path matching works for bundles too.
        let by_normalized: HashMap<(String, String), (i64, String, bool)> = rows
            .into_iter()
            .map(|(id, _path, normalized, method, yaml, deprecated)| {
                ((normalized, method), (id, yaml, deprecated))
            })
            .collect();
        for (path, method) in endpoints {
            let normalized = crate::openapi::lookup_path(api_type, path);
            if let Some(details) = by_normalized.get(&(normalized, method.clone())) {
                result.insert((path.clone(), method.clone()), details.clone());
            }
        }

        Ok(result)
    }

    async fn record_dependency(
        &self,
        params: RecordDependencyParams<'_>,
    ) -> Result<(), RepositoryError> {
        self.record_dependencies_bulk(vec![params], Vec::new())
            .await
    }

    #[instrument(skip_all)]
    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<(), RepositoryError> {
        if params.is_empty() {
            return self.insert_webhook_events(&outbox).await;
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // One array bind per column, so the statement stays fixed whatever the
        // number of rows.
        let client_ids: Vec<i64> = params.iter().map(|p| p.client_id).collect();
        let spec_version_ids: Vec<i64> = params.iter().map(|p| p.spec_version_id).collect();
        let api_types: Vec<&str> = params.iter().map(|p| p.api_type.as_str()).collect();
        let paths: Vec<&str> = params.iter().map(|p| p.path).collect();
        let normalized_paths: Vec<&str> = params.iter().map(|p| p.normalized_path).collect();
        let methods: Vec<&str> = params.iter().map(|p| p.method).collect();
        sqlx::query(
            "INSERT INTO dependencies (client_id, spec_version_id, api_type, path, normalized_path, method, last_seen_at) \
             SELECT r.client_id, r.spec_version_id, r.api_type, r.path, r.normalized_path, r.method, $7 \
             FROM UNNEST($1::BIGINT[], $2::BIGINT[], $3::TEXT[], $4::TEXT[], $5::TEXT[], $6::TEXT[]) \
                 AS r(client_id, spec_version_id, api_type, path, normalized_path, method) \
             ON CONFLICT(client_id, spec_version_id, api_type, path, method) DO UPDATE SET last_seen_at = EXCLUDED.last_seen_at",
        )
        .bind(client_ids)
        .bind(spec_version_ids)
        .bind(api_types)
        .bind(paths)
        .bind(normalized_paths)
        .bind(methods)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        write_outbox(&mut tx, &outbox).await?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn get_report(&self) -> Result<DependencyReport, RepositoryError> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Every recorded Pin, with the pinned version's current stability and
        // whether the endpoint still exists in that version (a snapshot
        // overwrite can drop an endpoint under a recorded dependency).
        type DepRow = (
            String,
            String,
            String,
            i32,
            i32,
            i32,
            String,
            String,
            String,
            bool,
            bool,
        );
        let dependency_rows: Vec<DepRow> = sqlx::query_as(
            r#"
            SELECT c.name, d.api_type, s.name, v.major, v.minor, v.patch, v.stability,
                   d.path, d.method,
                   COALESCE(e.deprecated, FALSE),
                   (e.id IS NOT NULL)
            FROM dependencies d
            JOIN clients c ON d.client_id = c.id
            JOIN spec_versions v ON d.spec_version_id = v.id
            JOIN services s ON v.service_id = s.id
            LEFT JOIN endpoints e ON e.spec_version_id = v.id AND e.api_type = d.api_type
                 AND e.normalized_path = d.normalized_path AND e.method = d.method
            "#,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let mut dependency_graph = Vec::new();
        let mut missing_endpoints = Vec::new();
        for (
            client,
            api_type,
            service,
            major,
            minor,
            patch,
            stability,
            path,
            method,
            deprecated,
            exists,
        ) in dependency_rows
        {
            let api_type = ApiType::from_str(&api_type).unwrap_or_default();
            let version = SemVer::new(major as u32, minor as u32, patch as u32);
            let stability: Stability = stability
                .parse()
                .map_err(|e: String| RepositoryError::Internal(e))?;
            if exists {
                dependency_graph.push(DependencyInfo {
                    api_type,
                    client,
                    service,
                    version,
                    stability,
                    path,
                    method,
                    deprecated,
                });
            } else {
                missing_endpoints.push(MissingEndpointInfo {
                    api_type,
                    client,
                    service,
                    version,
                    path,
                    method,
                });
            }
        }

        // Endpoints nobody requires, per version-line entry.
        type UnusedRow = (String, String, i32, i32, i32, String, String, String, bool);
        let unused_rows: Vec<UnusedRow> = sqlx::query_as(
            r#"
            SELECT s.name, e.api_type, v.major, v.minor, v.patch, v.stability, e.path, e.method, e.deprecated
            FROM endpoints e
            JOIN spec_versions v ON e.spec_version_id = v.id
            JOIN services s ON v.service_id = s.id
            WHERE NOT EXISTS (
                SELECT 1 FROM dependencies d
                WHERE d.spec_version_id = v.id AND d.api_type = e.api_type
                  AND d.normalized_path = e.normalized_path AND d.method = e.method
            )
            "#,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let unused_endpoints = unused_rows
            .into_iter()
            .map(
                |(service, api_type, major, minor, patch, stability, path, method, deprecated)| {
                    Ok(EndpointInfo {
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        service,
                        version: SemVer::new(major as u32, minor as u32, patch as u32),
                        stability: stability
                            .parse()
                            .map_err(|e: String| RepositoryError::Internal(e))?,
                        path,
                        method,
                        deprecated,
                    })
                },
            )
            .collect::<Result<Vec<_>, RepositoryError>>()?;

        Ok(DependencyReport {
            trunk_graph: Vec::new(),
            harvested_subscriptions: Vec::new(),
            peripheral_dependencies: Vec::new(),
            systems: BTreeMap::new(),
            trunk_stale_before: None,
            scope_label: None,
            unused_endpoints,
            missing_endpoints,
            dependency_graph,
            service_tags: HashMap::new(),
        })
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_version_origin")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM services")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM clients")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    /// Delete every user who does not hold the `admin` role.
    ///
    /// Keys on the role grant rather than the flag it replaced, so the set it
    /// spares is the same set the permission checks treat as administrators.
    async fn delete_all_non_admin_users(
        &self,
        spare_usernames: &[String],
    ) -> Result<u64, RepositoryError> {
        // A user survives the purge when they are an effective administrator:
        // direct `admin` role, `admin` via group membership, or a
        // configuration-held root username (`spare_usernames`).
        macro_rules! doomed_sql {
            () => {
                "SELECT id FROM users \
             WHERE NOT EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = users.id AND r.role = 'admin') \
             AND NOT EXISTS (SELECT 1 FROM user_group_members m \
                 JOIN user_group_roles gr ON gr.group_id = m.group_id AND gr.role = 'admin' \
                 WHERE m.user_id = users.id) \
             AND users.username <> ALL($1)"
            };
        }
        for statement in [
            concat!(
                "DELETE FROM sessions WHERE user_id IN (",
                doomed_sql!(),
                ")"
            ),
            concat!(
                "DELETE FROM api_tokens WHERE user_id IN (",
                doomed_sql!(),
                ")"
            ),
        ] {
            sqlx::query(statement)
                .bind(spare_usernames)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        let result = sqlx::query(concat!(
            "DELETE FROM users WHERE id IN (",
            doomed_sql!(),
            ")"
        ))
        .bind(spare_usernames)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("DELETE FROM dependencies")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_version_origin")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Release cuts are instance data too: without this the branch list and
        // the graph selector survive a factory reset as empty shells whose
        // names stay claimed. Their pin rows go with the participants.
        sqlx::query("DELETE FROM sanshain_branches")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM peripheral_dependencies")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM system_context")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM webhook_deliveries")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM webhook_outbox")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM webhooks")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM trunk_member_versions")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM channel_message_contracts")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM services")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM clients")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM service_tags")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Delete all sessions except the current admin's
        if let Some(uid) = keep_user_id {
            sqlx::query("DELETE FROM sessions WHERE user_id != $1")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens WHERE user_id != $1")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users WHERE id != $1")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        } else {
            sqlx::query("DELETE FROM sessions")
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens")
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users")
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn delete_producer(&self, name: &str) -> Result<bool, RepositoryError> {
        // spec_versions cascade endpoints and dependencies via FK; deleting
        // explicitly keeps both backends behaving identically.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = $1")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let service_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        sqlx::query(
            "DELETE FROM dependencies WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = $1)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "DELETE FROM endpoints WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = $1)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "DELETE FROM spec_version_origin WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = $1)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions WHERE service_id = $1")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // #26B: close this producer's open trunk pins so it leaves the current
        // main graph while its closed history survives (denormalized names, no
        // longer cascaded away). The stamp matches now_iso()'s shape.
        sqlx::query(
            "UPDATE trunk_dependencies SET valid_to = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             WHERE service_id = $1 AND valid_to IS NULL",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Its trunk member versions close the same way.
        sqlx::query(
            "UPDATE trunk_member_versions SET valid_to = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             WHERE service_id = $1 AND valid_to IS NULL",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM services WHERE id = $1")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(true)
    }

    async fn delete_consumer(&self, name: &str) -> Result<bool, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM clients WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let client_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        sqlx::query("DELETE FROM dependencies WHERE client_id = $1")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // #26B: close this consumer's open trunk pins; closed rows stay as
        // timeline history (denormalized names, no longer cascaded away).
        sqlx::query(
            "UPDATE trunk_dependencies SET valid_to = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             WHERE client_id = $1 AND valid_to IS NULL",
        )
        .bind(client_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(true)
    }

    async fn list_producers(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM services ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        let rows: Vec<(String, Option<String>, Option<String>, bool)> =
            sqlx::query_as("SELECT name, icon, domain, external FROM services ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(name, icon, domain, external)| ProducerSummary {
                name,
                versions: Vec::new(),
                is_favorite: false,
                icon,
                domain,
                external,
            })
            .collect())
    }

    async fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE services SET icon = $1, domain = $2 WHERE name = $3")
            .bind(icon)
            .bind(domain)
            .bind(service_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn set_producer_external(
        &self,
        service_name: &str,
        external: bool,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query("UPDATE services SET external = $1 WHERE name = $2")
            .bind(external)
            .bind(service_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_consumers(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT c.name FROM clients c INNER JOIN dependencies d ON d.client_id = c.id ORDER BY c.name"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_consumer_endpoints(
        &self,
        client_name: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        type ClientEndpointRow = (
            String,
            String,
            i32,
            i32,
            i32,
            String,
            String,
            String,
            Option<String>,
            bool,
        );
        let rows: Vec<ClientEndpointRow> = sqlx::query_as(
            "SELECT d.api_type, s.name, v.major, v.minor, v.patch, v.stability, d.path, d.method, e.yaml_content, \
             COALESCE(e.deprecated, FALSE) \
             FROM dependencies d \
             JOIN clients c ON d.client_id = c.id \
             JOIN spec_versions v ON d.spec_version_id = v.id \
             JOIN services s ON v.service_id = s.id \
             LEFT JOIN endpoints e ON e.spec_version_id = v.id AND e.api_type = d.api_type \
                  AND e.normalized_path = d.normalized_path AND e.method = d.method \
             WHERE c.name = $1 \
             ORDER BY s.name, d.path, d.method",
        )
        .bind(client_name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(
                |(
                    api_type,
                    service,
                    major,
                    minor,
                    patch,
                    stability,
                    path,
                    method,
                    yaml_content,
                    deprecated,
                )| {
                    Ok(ConsumerEndpointInfo {
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        service,
                        version: SemVer::new(major as u32, minor as u32, patch as u32),
                        stability: stability
                            .parse()
                            .map_err(|e: String| RepositoryError::Internal(e))?,
                        path,
                        method,
                        yaml_content,
                        deprecated,
                    })
                },
            )
            .collect()
    }

    async fn search_endpoints(
        &self,
        search: &EndpointSearch,
    ) -> Result<Vec<EndpointSearchHit>, RepositoryError> {
        type HitRow = (
            String,
            String,
            i32,
            i32,
            i32,
            String,
            String,
            String,
            bool,
            i64,
            String,
        );
        let pattern = crate::infrastructure::like_pattern(&search.query);
        let method = search.method.as_ref().map(|m| m.to_uppercase());
        let api_type = search.api_type.map(|t| t.as_str());
        let rows: Vec<HitRow> = sqlx::query_as(
            "SELECT s.name, v.api_type, v.major, v.minor, v.patch, v.stability, e.method, e.path, \
             e.deprecated, \
             (SELECT COUNT(DISTINCT d.client_id) FROM dependencies d \
              WHERE d.spec_version_id = v.id AND d.api_type = e.api_type \
                AND d.normalized_path = e.normalized_path AND d.method = e.method), \
             e.yaml_content \
             FROM endpoints e \
             JOIN spec_versions v ON v.id = e.spec_version_id \
             JOIN services s ON s.id = v.service_id \
             WHERE (LOWER(e.path) LIKE $1 ESCAPE '\\' OR LOWER(e.method) LIKE $1 ESCAPE '\\') \
               AND ($2::text IS NULL OR UPPER(e.method) = $2) \
               AND ($3::text IS NULL OR v.api_type = $3) \
               AND ($4 OR NOT EXISTS (SELECT 1 FROM spec_versions o \
                    WHERE o.service_id = v.service_id AND o.api_type = v.api_type \
                      AND (o.major > v.major OR (o.major = v.major AND o.minor > v.minor) \
                           OR (o.major = v.major AND o.minor = v.minor AND o.patch > v.patch)))) \
             ORDER BY s.name, v.api_type, v.major, v.minor, v.patch, e.path, e.method",
        )
        .bind(&pattern)
        .bind(&method)
        .bind(api_type)
        .bind(search.all_versions)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(
                |(
                    producer,
                    api_type,
                    major,
                    minor,
                    patch,
                    stability,
                    method,
                    path,
                    deprecated,
                    consumer_count,
                    yaml_content,
                )| {
                    Ok(EndpointSearchHit {
                        producer,
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        version: SemVer::new(major as u32, minor as u32, patch as u32),
                        stability: stability
                            .parse()
                            .map_err(|e: String| RepositoryError::Internal(e))?,
                        method,
                        path,
                        deprecated,
                        consumer_count,
                        summary: None,
                        yaml_content,
                    })
                },
            )
            .collect()
    }

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0)
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        let row: Option<(i64, String, String, bool)> = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(|(id, username, password_hash, approved)| User {
            id,
            username,
            password_hash,
            approved,
        }))
    }

    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        approved: bool,
    ) -> Result<User, RepositoryError> {
        sqlx::query("INSERT INTO users (username, password_hash, approved) VALUES ($1, $2, $3)")
            .bind(username)
            .bind(password_hash)
            .bind(approved)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64, String, String, bool) = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users WHERE username = $1",
        )
        .bind(username)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(User {
            id: row.0,
            username: row.1,
            password_hash: row.2,
            approved: row.3,
        })
    }

    async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
        let rows: Vec<(i64, String, String, bool)> = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(id, username, password_hash, approved)| User {
                id,
                username,
                password_hash,
                approved,
            })
            .collect())
    }

    async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let result =
            sqlx::query("UPDATE users SET approved = TRUE WHERE id = $1 AND approved = FALSE")
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(new_hash)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        use rand::RngExt;
        let mut token_bytes = [0u8; 32];
        rand::rng().fill(&mut token_bytes);
        let token = hex::encode(token_bytes);
        self.create_session_with_token(user_id, &token, expires_at)
            .await
    }

    async fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        let token_hash = hash_session_token(token);
        sqlx::query(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, $3::timestamp)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(Session {
            token: token.to_string(),
            user_id,
            expires_at: expires_at.to_string(),
        })
    }

    async fn validate_session(
        &self,
        token: &str,
    ) -> Result<Option<(User, Session)>, RepositoryError> {
        let row: Option<(i64, i64, String, String, String, bool)> = sqlx::query_as(
            r#"
            SELECT s.user_id, u.id, u.username, u.password_hash, s.expires_at::text, u.approved
            FROM sessions s
            JOIN users u ON s.user_id = u.id
            WHERE s.token = $1 AND s.expires_at > NOW()
            "#,
        )
        .bind(hash_session_token(token))
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(
            |(_, user_id, username, password_hash, expires_at, approved)| {
                let user = User {
                    id: user_id,
                    username,
                    password_hash,
                    approved,
                };
                let session = Session {
                    token: token.to_string(),
                    user_id,
                    expires_at,
                };
                (user, session)
            },
        ))
    }

    async fn delete_expired_credentials(&self, now_iso: &str) -> Result<u64, RepositoryError> {
        // The two columns have different types — `sessions.expires_at` is a
        // TIMESTAMP, `api_tokens.expires_at` RFC 3339 text — and the cutoff
        // arrives as text: each comparison casts to an instant. A failure on
        // one table must not keep the other from being cleaned.
        let statements = [
            "DELETE FROM sessions WHERE expires_at < $1::timestamptz",
            "DELETE FROM api_tokens WHERE expires_at::timestamptz < $1::timestamptz",
        ];
        let mut removed = 0u64;
        let mut failure = None;
        for statement in statements {
            match sqlx::query(statement)
                .bind(now_iso)
                .execute(&self.pool)
                .await
            {
                Ok(res) => removed += res.rows_affected(),
                Err(e) => failure = Some(RepositoryError::Internal(e.to_string())),
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(removed),
        }
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(hash_session_token(token))
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_user_sign_in(
        &self,
        user_id: i64,
        method: SignInMethod,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO user_profiles (user_id, last_sign_in) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET last_sign_in = excluded.last_sign_in",
        )
        .bind(user_id)
        .bind(method.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn set_display_name(
        &self,
        user_id: i64,
        display_name: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO user_profiles (user_id, display_name) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET display_name = excluded.display_name",
        )
        .bind(user_id)
        .bind(display_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn set_avatar_hash(
        &self,
        user_id: i64,
        avatar_hash: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO user_profiles (user_id, avatar_hash) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET avatar_hash = excluded.avatar_hash",
        )
        .bind(user_id)
        .bind(avatar_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn set_own_avatar_hash(
        &self,
        user_id: i64,
        own_avatar_hash: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO user_profiles (user_id, own_avatar_hash) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET own_avatar_hash = excluded.own_avatar_hash",
        )
        .bind(user_id)
        .bind(own_avatar_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn set_directory_id(
        &self,
        user_id: i64,
        directory_id: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO user_profiles (user_id, directory_id) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET directory_id = excluded.directory_id",
        )
        .bind(user_id)
        .bind(directory_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn find_user_by_directory_id(
        &self,
        directory_id: &str,
    ) -> Result<Option<User>, RepositoryError> {
        let row: Option<(i64, String, String, bool)> = sqlx::query_as(
            "SELECT u.id, u.username, u.password_hash, u.approved FROM users u \
             JOIN user_profiles p ON p.user_id = u.id WHERE p.directory_id = $1",
        )
        .bind(directory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|(id, username, password_hash, approved)| User {
            id,
            username,
            password_hash,
            approved,
        }))
    }

    async fn rename_user(&self, user_id: i64, username: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE users SET username = $1 WHERE id = $2")
            .bind(username)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn merge_user_into(&self, from: i64, into: i64) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE user_roles SET user_id = $1 WHERE user_id = $2 \
             AND role NOT IN (SELECT role FROM user_roles WHERE user_id = $3)",
        )
        .bind(into)
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE user_group_members SET user_id = $1 WHERE user_id = $2 \
             AND group_id NOT IN (SELECT group_id FROM user_group_members WHERE user_id = $3)",
        )
        .bind(into)
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE producer_user_maintainers SET user_id = $1 WHERE user_id = $2 \
             AND service_id NOT IN (SELECT service_id FROM producer_user_maintainers WHERE user_id = $3)",
        )
        .bind(into)
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE user_favorites SET user_id = $1 WHERE user_id = $2 \
             AND NOT EXISTS (SELECT 1 FROM user_favorites f WHERE f.user_id = $3 \
             AND f.item_type = user_favorites.item_type AND f.item_name = user_favorites.item_name)",
        )
        .bind(into)
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for statement in [
            "UPDATE api_tokens SET user_id = $1 WHERE user_id = $2",
            "UPDATE sessions SET user_id = $1 WHERE user_id = $2",
        ] {
            sqlx::query(statement)
                .bind(into)
                .bind(from)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(from)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_user_profiles(&self) -> Result<Vec<UserProfile>, RepositoryError> {
        let rows: Vec<UserProfileRow> = sqlx::query_as(
            "SELECT user_id, last_sign_in, display_name, avatar_hash, directory_id, own_avatar_hash FROM user_profiles",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(user_profile_from_row).collect())
    }

    async fn get_user_profile(&self, user_id: i64) -> Result<Option<UserProfile>, RepositoryError> {
        let row: Option<UserProfileRow> = sqlx::query_as(
            "SELECT user_id, last_sign_in, display_name, avatar_hash, directory_id, own_avatar_hash FROM user_profiles WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(user_profile_from_row))
    }

    async fn set_session_sign_in(
        &self,
        token: &str,
        method: SignInMethod,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE sessions SET sign_in = $1 WHERE token = $2")
            .bind(method.as_str())
            .bind(hash_session_token(token))
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn session_sign_in(&self, token: &str) -> Result<Option<SignInMethod>, RepositoryError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT sign_in FROM sessions WHERE token = $1")
                .bind(hash_session_token(token))
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row
            .and_then(|(value,)| value)
            .and_then(|value| SignInMethod::parse(&value)))
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    // --- API Tokens ---

    async fn create_api_token(&self, token: NewApiToken<'_>) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO api_tokens (id, user_id, name, token_hash, scope, created_at, expires_at, issued_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(token.id)
            .bind(token.user_id)
            .bind(token.name)
            .bind(token.token_hash)
            .bind(token.scope.as_str())
            .bind(token.created_at)
            .bind(token.expires_at)
            .bind(token.issued_by)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("duplicate key") || msg.contains("unique constraint") {
                    RepositoryError::Conflict
                } else {
                    RepositoryError::Internal(msg)
                }
            })?;
        Ok(())
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        let rows: Vec<ApiTokenRow> = sqlx::query_as(
            "SELECT id, user_id, name, token_hash, scope, created_at, expires_at, last_used_at, issued_by FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(ApiToken::from).collect())
    }

    async fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2")
            .bind(token_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn validate_api_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiTokenIdentity>, RepositoryError> {
        type Row = (String, String, Option<String>, i64, String, String, bool);
        let row: Option<Row> = sqlx::query_as(
            r#"
            SELECT t.id, t.scope, t.last_used_at, u.id, u.username, u.password_hash, u.approved
            FROM api_tokens t
            JOIN users u ON t.user_id = u.id
            WHERE t.token_hash = $1 AND t.expires_at::timestamptz > NOW()
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(
            |(token_id, scope, last_used_at, id, username, password_hash, approved)| {
                ApiTokenIdentity {
                    token_id,
                    scope: ApiTokenScope::parse(&scope).unwrap_or(ApiTokenScope::ReadOnly),
                    last_used_at,
                    user: User {
                        id,
                        username,
                        password_hash,
                        approved,
                    },
                }
            },
        ))
    }

    async fn touch_api_token(&self, token_id: &str, used_at: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE api_tokens SET last_used_at = $1 WHERE id = $2")
            .bind(used_at)
            .bind(token_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let result = sqlx::query("DELETE FROM dependencies WHERE last_seen_at < $1")
            .bind(cutoff_iso)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn grant_user_role(&self, user_id: i64, role: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO user_roles (user_id, role) VALUES ($1, $2) ON CONFLICT (user_id, role) DO NOTHING")
            .bind(user_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn revoke_user_role(&self, user_id: i64, role: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM user_roles WHERE user_id = $1 AND role = $2")
            .bind(user_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_user_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT role FROM user_roles WHERE user_id = $1 ORDER BY role")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn effective_stored_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT role FROM user_roles WHERE user_id = $1
             UNION
             SELECT gr.role FROM user_group_roles gr
               JOIN user_group_members gm ON gm.group_id = gr.group_id
             WHERE gm.user_id = $2
             ORDER BY role",
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> Result<Group, RepositoryError> {
        sqlx::query("INSERT INTO user_groups (name, source) VALUES ($1, $2) ON CONFLICT (name, source) DO NOTHING")
            .bind(name)
            .bind(source.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let row: (i64,) =
            sqlx::query_as("SELECT id FROM user_groups WHERE name = $1 AND source = $2")
                .bind(name)
                .bind(source.as_str())
                .fetch_one(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(Group {
            id: row.0,
            name: name.to_string(),
            source,
        })
    }

    async fn rename_group(&self, group_id: i64, name: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("UPDATE user_groups SET name = $1 WHERE id = $2")
            .bind(name)
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_group(&self, group_id: i64) -> Result<bool, RepositoryError> {
        // Cascade would handle these, but removing them explicitly keeps the
        // two backends behaving identically.
        sqlx::query("DELETE FROM user_group_members WHERE group_id = $1")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM user_group_roles WHERE group_id = $1")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM user_groups WHERE id = $1")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_groups(&self) -> Result<Vec<Group>, RepositoryError> {
        let rows: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, name, source FROM user_groups ORDER BY source, name")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, name, source)| {
                GroupSource::parse(&source).map(|source| Group { id, name, source })
            })
            .collect())
    }

    async fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM user_group_roles WHERE group_id = $1")
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for role in roles {
            sqlx::query("INSERT INTO user_group_roles (group_id, role) VALUES ($1, $2) ON CONFLICT (group_id, role) DO NOTHING")
                .bind(group_id)
                .bind(role)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_group_roles(&self, group_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT role FROM user_group_roles WHERE group_id = $1 ORDER BY role")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO user_group_members (group_id, user_id) VALUES ($1, $2) ON CONFLICT (group_id, user_id) DO NOTHING")
            .bind(group_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result =
            sqlx::query("DELETE FROM user_group_members WHERE group_id = $1 AND user_id = $2")
                .bind(group_id)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_group_member_ids(&self, group_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM user_group_members WHERE group_id = $1 ORDER BY user_id",
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    // --- Maintainer Scope ---

    async fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO producer_user_maintainers (service_id, user_id) VALUES ($1, $2) ON CONFLICT (service_id, user_id) DO NOTHING",
        )
        .bind(service_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "DELETE FROM producer_user_maintainers WHERE service_id = $1 AND user_id = $2",
        )
        .bind(service_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO producer_group_maintainers (service_id, group_id) VALUES ($1, $2) ON CONFLICT (service_id, group_id) DO NOTHING",
        )
        .bind(service_id)
        .bind(group_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "DELETE FROM producer_group_maintainers WHERE service_id = $1 AND group_id = $2",
        )
        .bind(service_id)
        .bind(group_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_user_maintainer_ids(&self, service_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM producer_user_maintainers WHERE service_id = $1 ORDER BY user_id",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT group_id FROM producer_group_maintainers WHERE service_id = $1 ORDER BY group_id",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_all_user_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        sqlx::query_as(
            "SELECT s.name, m.user_id FROM producer_user_maintainers m \
             JOIN services s ON s.id = m.service_id ORDER BY s.name, m.user_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))
    }

    async fn list_all_group_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        sqlx::query_as(
            "SELECT s.name, m.group_id FROM producer_group_maintainers m \
             JOIN services s ON s.id = m.service_id ORDER BY s.name, m.group_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))
    }

    async fn list_group_maintained_producers(
        &self,
        group_ids: &[i64],
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT s.name FROM producer_group_maintainers pgm \
             JOIN services s ON s.id = pgm.service_id \
             WHERE pgm.group_id = ANY($1) ORDER BY s.name",
        )
        .bind(group_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> Result<bool, RepositoryError> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM (
                 SELECT 1 FROM producer_user_maintainers
                  WHERE service_id = $1 AND user_id = $2
                 UNION ALL
                 SELECT 1 FROM producer_group_maintainers pgm
                   JOIN user_group_members gm ON gm.group_id = pgm.group_id
                  WHERE pgm.service_id = $3 AND gm.user_id = $4
             ) AS assignments",
        )
        .bind(service_id)
        .bind(user_id)
        .bind(service_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0 > 0)
    }

    async fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT s.name FROM services s
               JOIN producer_user_maintainers m ON m.service_id = s.id
              WHERE m.user_id = $1
             UNION
             SELECT s.name FROM services s
               JOIN producer_group_maintainers pgm ON pgm.service_id = s.id
               JOIN user_group_members gm ON gm.group_id = pgm.group_id
              WHERE gm.user_id = $2
             ORDER BY name",
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        for tag in tags {
            sqlx::query("INSERT INTO service_tags (service_id, tag) VALUES ($1, $2) ON CONFLICT (service_id, tag) DO NOTHING")
                .bind(service_id)
                .bind(tag)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    async fn remove_service_tag(&self, service_id: i64, tag: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM service_tags WHERE service_id = $1 AND tag = $2")
            .bind(service_id)
            .bind(tag)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn close_trunk_pins_for_service_api(
        &self,
        service_id: i64,
        api_type: ApiType,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        let res = sqlx::query(
            "UPDATE trunk_dependencies SET valid_to = $1 \
             WHERE service_id = $2 AND api_type = $3 AND valid_to IS NULL",
        )
        .bind(now_iso)
        .bind(service_id)
        .bind(api_type.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE trunk_member_versions SET valid_to = $1 \
             WHERE service_id = $2 AND api_type = $3 AND valid_to IS NULL",
        )
        .bind(now_iso)
        .bind(service_id)
        .bind(api_type.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(res.rows_affected())
    }

    async fn get_all_service_tags(&self) -> Result<HashMap<String, Vec<String>>, RepositoryError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT s.name, st.tag FROM service_tags st JOIN services s ON s.id = st.service_id ORDER BY s.name, st.tag"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let mut result: HashMap<String, Vec<String>> = HashMap::new();
        for (name, tag) in rows {
            result.entry(name).or_default().push(tag);
        }
        Ok(result)
    }

    async fn get_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT owner_service_id, payload_yaml FROM channel_message_contracts WHERE channel = $1 AND message_name = $2",
        )
        .bind(channel)
        .bind(message_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(
            row.map(|(owner_service_id, payload_yaml)| ChannelMessageContract {
                channel: channel.to_string(),
                message_name: message_name.to_string(),
                owner_service_id,
                payload_yaml,
            }),
        )
    }

    async fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO channel_message_contracts (channel, message_name, owner_service_id, payload_yaml) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (channel, message_name) \
             DO UPDATE SET owner_service_id = EXCLUDED.owner_service_id, payload_yaml = EXCLUDED.payload_yaml",
        )
        .bind(&contract.channel)
        .bind(&contract.message_name)
        .bind(contract.owner_service_id)
        .bind(&contract.payload_yaml)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn delete_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "DELETE FROM channel_message_contracts WHERE channel = $1 AND message_name = $2",
        )
        .bind(channel)
        .bind(message_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_channel_message_contracts(
        &self,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
            "SELECT channel, message_name, owner_service_id, payload_yaml FROM channel_message_contracts ORDER BY channel, message_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(channel, message_name, owner_service_id, payload_yaml)| ChannelMessageContract {
                    channel,
                    message_name,
                    owner_service_id,
                    payload_yaml,
                },
            )
            .collect())
    }

    async fn reconcile_harvested_subscriptions(
        &self,
        client_id: i64,
        client_name: &str,
        trunk: bool,
        subs: &[HarvestedSubInput],
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // The client's current open subscriptions, to diff against `subs`.
        let open: Vec<(String, String, Option<i64>, bool)> = sqlx::query_as(
            "SELECT channel, message_name, owner_service_id, trunk \
             FROM harvested_subscriptions WHERE client_id = $1 AND valid_to IS NULL",
        )
        .bind(client_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Close every open row that is gone, or whose owner/trunk changed.
        for (channel, message_name, owner, row_trunk) in &open {
            let keep = subs.iter().any(|s| {
                &s.channel == channel
                    && &s.message_name == message_name
                    && &s.owner_service_id == owner
                    && *row_trunk == trunk
            });
            if !keep {
                sqlx::query(
                    "UPDATE harvested_subscriptions SET valid_to = $1 \
                     WHERE client_id = $2 AND channel = $3 AND message_name = $4 AND valid_to IS NULL",
                )
                .bind(now_iso)
                .bind(client_id)
                .bind(channel)
                .bind(message_name)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }

        // Open a fresh row for every desired subscription that has no unchanged
        // open row (new, or reopened after an owner/trunk change above).
        for s in subs {
            let unchanged = open.iter().any(|(c, m, o, t)| {
                c == &s.channel && m == &s.message_name && o == &s.owner_service_id && *t == trunk
            });
            if unchanged {
                continue;
            }
            sqlx::query(
                "INSERT INTO harvested_subscriptions \
                   (client_id, client_name, channel, message_name, owner_service_id, owner_name, trunk, valid_from) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (client_id, channel, message_name) WHERE valid_to IS NULL DO NOTHING",
            )
            .bind(client_id)
            .bind(client_name)
            .bind(&s.channel)
            .bind(&s.message_name)
            .bind(s.owner_service_id)
            .bind(&s.owner_name)
            .bind(trunk)
            .bind(now_iso)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_current_harvested_subscriptions(
        &self,
    ) -> Result<Vec<HarvestedSubscription>, RepositoryError> {
        let rows: Vec<(String, String, String, Option<String>, bool)> = sqlx::query_as(
            "SELECT client_name, channel, message_name, owner_name, trunk \
             FROM harvested_subscriptions WHERE valid_to IS NULL \
             ORDER BY client_name, channel, message_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(
                |(client, channel, message_name, owner, trunk)| HarvestedSubscription {
                    client,
                    channel,
                    message_name,
                    owner,
                    trunk,
                },
            )
            .collect())
    }

    async fn reconcile_periphery(
        &self,
        system: &str,
        trunk: bool,
        resources: &[String],
        now_iso: &str,
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<bool, RepositoryError> {
        let mut tracks = vec![PeripheryTrack::Dev];
        if trunk {
            tracks.push(PeripheryTrack::Trunk);
        }
        let mut changed = false;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for track in tracks {
            let (track, _) = periphery_track_key(track);
            let open: Vec<(String,)> = sqlx::query_as(
                "SELECT resource_ref FROM peripheral_dependencies \
                 WHERE track = $1 AND branch_id IS NULL AND system_name = $2 AND valid_to IS NULL",
            )
            .bind(track)
            .bind(system)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            for (resource,) in &open {
                if !resources.contains(resource) {
                    sqlx::query(
                        "UPDATE peripheral_dependencies SET valid_to = $1 \
                         WHERE track = $2 AND branch_id IS NULL AND system_name = $3 \
                           AND resource_ref = $4 AND valid_to IS NULL",
                    )
                    .bind(now_iso)
                    .bind(track)
                    .bind(system)
                    .bind(resource)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    changed = true;
                }
            }
            for resource in resources {
                if open.iter().any(|(r,)| r == resource) {
                    continue;
                }
                sqlx::query(
                    "INSERT INTO peripheral_dependencies \
                       (track, branch_id, system_name, resource_ref, valid_from) \
                     VALUES ($1, NULL, $2, $3, $4) \
                     ON CONFLICT (track, (COALESCE(branch_id, 0)), system_name, resource_ref) \
                     WHERE valid_to IS NULL DO NOTHING",
                )
                .bind(track)
                .bind(system)
                .bind(resource)
                .bind(now_iso)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                changed = true;
            }
        }
        write_outbox(&mut tx, &outbox).await?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(changed)
    }

    async fn list_periphery(
        &self,
        track: PeripheryTrack,
        at: Option<&str>,
    ) -> Result<Vec<PeripheralDependency>, RepositoryError> {
        let (track, branch_id) = periphery_track_key(track);
        macro_rules! base_sql {
            () => {
                "SELECT system_name, resource_ref FROM peripheral_dependencies \
                    WHERE track = $1 AND COALESCE(branch_id, 0) = COALESCE($2::BIGINT, 0)"
            };
        }
        macro_rules! order_sql {
            () => {
                "ORDER BY system_name, resource_ref"
            };
        }
        let rows: Vec<(String, String)> = match at {
            Some(at) => {
                sqlx::query_as(concat!(
                    base_sql!(),
                    " AND valid_from <= $3 AND (valid_to IS NULL OR valid_to > $3) ",
                    order_sql!()
                ))
                .bind(track)
                .bind(branch_id)
                .bind(at)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as(concat!(base_sql!(), " AND valid_to IS NULL ", order_sql!()))
                    .bind(track)
                    .bind(branch_id)
                    .fetch_all(&self.pool)
                    .await
            }
        }
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|(system, resource)| PeripheralDependency { system, resource })
            .collect())
    }

    async fn upsert_system_context(
        &self,
        system: &str,
        context: &SystemContext,
        now_iso: &str,
    ) -> Result<bool, RepositoryError> {
        if self.get_system_context(system).await?.as_ref() == Some(context) {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO system_context \
                   (system_name, owner_kind, owner_name, description, source_location, links_json, tags_json, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (system_name) DO UPDATE SET \
                   owner_kind = EXCLUDED.owner_kind, owner_name = EXCLUDED.owner_name, \
                   description = EXCLUDED.description, source_location = EXCLUDED.source_location, \
                   links_json = EXCLUDED.links_json, tags_json = EXCLUDED.tags_json, updated_at = EXCLUDED.updated_at",
        )
        .bind(system)
        .bind(context.owner.as_ref().map(|o| o.kind.as_str()))
        .bind(context.owner.as_ref().map(|o| o.name.as_str()))
        .bind(context.description.as_deref())
        .bind(context.source_location.as_deref())
        .bind(links_json(context)?)
        .bind(tags_json(context)?)
        .bind(now_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(true)
    }

    async fn get_system_context(
        &self,
        system: &str,
    ) -> Result<Option<SystemContext>, RepositoryError> {
        let row: Option<SystemContextRow> = sqlx::query_as(
            "SELECT system_name, owner_kind, owner_name, description, source_location, links_json, tags_json \
             FROM system_context WHERE system_name = $1",
        )
        .bind(system)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        row.map(|row| system_context_from_row(row).map(|(_, c)| c))
            .transpose()
    }

    async fn list_system_contexts(
        &self,
    ) -> Result<BTreeMap<String, SystemContext>, RepositoryError> {
        let rows: Vec<SystemContextRow> = sqlx::query_as(
            "SELECT system_name, owner_kind, owner_name, description, source_location, links_json, tags_json \
             FROM system_context ORDER BY system_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(system_context_from_row).collect()
    }

    async fn delete_system_context(&self, system: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM system_context WHERE system_name = $1")
            .bind(system)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_member_versions(
        &self,
        track: MemberTrack,
        at: Option<&str>,
    ) -> Result<Vec<MemberVersion>, RepositoryError> {
        type Row = (String, String, i32, i32, i32, String, Option<String>);
        // Trunk binds a placeholder branch id it never reads, so both stores
        // share one placeholder numbering.
        macro_rules! trunk {
            () => {
                "SELECT service_name, api_type, major, minor, patch, valid_from, last_provided_at \
                 FROM trunk_member_versions WHERE $1::BIGINT IS NULL"
            };
        }
        macro_rules! branch {
            () => {
                "SELECT service_name, api_type, major, minor, patch, valid_from, NULL::TEXT \
                 FROM branch_member_versions WHERE branch_id = $1"
            };
        }
        macro_rules! as_of {
            () => {
                " AND valid_from <= $2 AND (valid_to IS NULL OR valid_to > $2) ORDER BY service_name, api_type"
            };
        }
        macro_rules! open {
            () => {
                " AND valid_to IS NULL ORDER BY service_name, api_type"
            };
        }
        let (sql, branch_id) = match (track, at) {
            (MemberTrack::Trunk, Some(_)) => (concat!(trunk!(), as_of!()), None),
            (MemberTrack::Trunk, None) => (concat!(trunk!(), open!()), None),
            (MemberTrack::Branch(id), Some(_)) => (concat!(branch!(), as_of!()), Some(id)),
            (MemberTrack::Branch(id), None) => (concat!(branch!(), open!()), Some(id)),
        };
        let mut query = sqlx::query_as::<_, Row>(sql).bind(branch_id);
        if let Some(at) = at {
            query = query.bind(at);
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(
                |(service, api_type, major, minor, patch, valid_from, last_provided_at)| {
                    Ok(MemberVersion {
                        service,
                        api_type: api_type
                            .parse()
                            .map_err(|e: String| RepositoryError::Internal(e))?,
                        version: SemVer::new(major as u32, minor as u32, patch as u32),
                        valid_from,
                        last_provided_at,
                    })
                },
            )
            .collect()
    }

    async fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> Result<(), RepositoryError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO audit_logs (timestamp, username, action, details, service, version, action_type, diff, stream, branch_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(timestamp)
        .bind(username)
        .bind(log.action)
        .bind(log.details)
        .bind(log.service)
        .bind(log.version)
        .bind(log.action_type)
        .bind(log.diff)
        .bind(log.stream)
        .bind(log.branch_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn get_audit_logs(
        &self,
        filter: AuditLogFilter,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        // One fixed statement: an absent filter binds NULL and matches every row.
        // Identity first: a resolved branch matches by id, so a rename does
        // not orphan its history. Only an unresolvable name falls back to the
        // recorded label — which is how a deleted branch stays reachable.
        const SQL: &str = "SELECT id, timestamp, username, action, details, service, version, action_type, diff, stream \
             FROM audit_logs \
             WHERE ($1::TEXT IS NULL OR timestamp >= $1) \
             AND ($2::TEXT IS NULL OR timestamp <= $2) \
             AND ($3::TEXT IS NULL OR action_type = $3) \
             AND ($4::TEXT IS NULL OR service LIKE $4) \
             AND ($5::TEXT IS NULL OR version LIKE $5) \
             AND CASE WHEN $6::BIGINT IS NOT NULL THEN branch_id = $6 \
                 ELSE ($7::TEXT IS NULL OR stream = $7) END \
             ORDER BY id DESC LIMIT $8";

        let query = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(SQL)
        .bind(filter.from_date)
        .bind(filter.to_date)
        .bind(filter.action_type)
        .bind(filter.service_wildcard)
        .bind(filter.version_wildcard)
        .bind(filter.branch_id)
        .bind(filter.stream)
        .bind(filter.limit as i64);

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    timestamp,
                    username,
                    action,
                    details,
                    service,
                    version,
                    action_type,
                    diff,
                    stream,
                )| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        username,
                        action,
                        details,
                        service,
                        version,
                        action_type,
                        diff,
                        stream,
                    }
                },
            )
            .collect())
    }

    async fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        let rows: Vec<AuditLogRow> = sqlx::query_as(
            "SELECT id, timestamp, username, action, details, service, version, action_type, diff, stream FROM audit_logs ORDER BY id DESC LIMIT $1"
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    timestamp,
                    username,
                    action,
                    details,
                    service,
                    version,
                    action_type,
                    diff,
                    stream,
                )| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        username,
                        action,
                        details,
                        service,
                        version,
                        action_type,
                        diff,
                        stream,
                    }
                },
            )
            .collect())
    }

    async fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT item_name FROM user_favorites WHERE user_id = $1 AND item_type = $2 ORDER BY item_name"
        )
        .bind(user_id)
        .bind(item_type)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO user_favorites (user_id, item_type, item_name) VALUES ($1, $2, $3) ON CONFLICT(user_id, item_type, item_name) DO NOTHING"
        )
        .bind(user_id)
        .bind(item_type)
        .bind(item_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "DELETE FROM user_favorites WHERE user_id = $1 AND item_type = $2 AND item_name = $3",
        )
        .bind(user_id)
        .bind(item_type)
        .bind(item_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    // --- Webhooks (#47, ADR-0009) ---

    async fn insert_webhook_events(
        &self,
        events: &[NewWebhookEvent],
    ) -> Result<(), RepositoryError> {
        if events.is_empty() {
            return Ok(());
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        write_outbox(&mut tx, events).await?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))
    }

    async fn list_current_consumers_of(
        &self,
        service_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT c.name FROM dependencies d \
             JOIN spec_versions sv ON sv.id = d.spec_version_id \
             JOIN clients c ON c.id = d.client_id \
             WHERE sv.service_id = $1 \
             UNION \
             SELECT DISTINCT t.client_name FROM trunk_dependencies t \
             WHERE t.service_id = $1 AND t.valid_to IS NULL \
             ORDER BY 1",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn create_webhook(&self, hook: NewWebhook<'_>) -> Result<i64, RepositoryError> {
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO webhooks (scope_kind, scope_name, url, format, event_types, producer_glob, secret, secret_hint, paused, allow_list_exempt, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, FALSE, $9, $10, $11) RETURNING id",
        )
        .bind(hook.scope.kind())
        .bind(hook.scope.name())
        .bind(hook.url)
        .bind(hook.format.as_str())
        .bind(encode_event_types(hook.event_types))
        .bind(hook.producer_glob)
        .bind(hook.secret)
        .bind(hook.secret_hint)
        .bind(hook.allow_list_exempt)
        .bind(hook.created_by)
        .bind(hook.created_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(id)
    }

    async fn list_webhooks(
        &self,
        scope: Option<&WebhookScope>,
    ) -> Result<Vec<Webhook>, RepositoryError> {
        let rows: Vec<WebhookRow> = match scope {
            Some(scope) => {
                sqlx::query_as(concat!(
                    "SELECT ",
                    webhook_columns!(),
                    " FROM webhooks h \
                     WHERE h.scope_kind = $1 AND COALESCE(h.scope_name, '') = COALESCE($2, '') \
                     ORDER BY h.id"
                ))
                .bind(scope.kind())
                .bind(scope.name())
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as(concat!(
                    "SELECT ",
                    webhook_columns!(),
                    " FROM webhooks h ORDER BY h.id"
                ))
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(webhook_from_row).collect()
    }

    async fn get_webhook(&self, id: i64) -> Result<Option<Webhook>, RepositoryError> {
        let row: Option<WebhookRow> = sqlx::query_as(concat!(
            "SELECT ",
            webhook_columns!(),
            " FROM webhooks h WHERE h.id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        row.map(webhook_from_row).transpose()
    }

    async fn update_webhook(&self, id: i64, patch: &WebhookPatch) -> Result<bool, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM webhooks WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        if exists.is_none() {
            return Ok(false);
        }
        if let Some(paused) = patch.paused {
            sqlx::query("UPDATE webhooks SET paused = $1 WHERE id = $2")
                .bind(paused)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        if let Some(types) = &patch.event_types {
            sqlx::query("UPDATE webhooks SET event_types = $1 WHERE id = $2")
                .bind(encode_event_types(types))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        if let Some(glob) = &patch.producer_glob {
            sqlx::query("UPDATE webhooks SET producer_glob = $1 WHERE id = $2")
                .bind(glob.as_deref())
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(true)
    }

    async fn rotate_webhook_secret(
        &self,
        id: i64,
        secret: &str,
        secret_hint: &str,
    ) -> Result<bool, RepositoryError> {
        let done = sqlx::query("UPDATE webhooks SET secret = $1, secret_hint = $2 WHERE id = $3")
            .bind(secret)
            .bind(secret_hint)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(done.rows_affected() > 0)
    }

    async fn delete_webhook(&self, id: i64) -> Result<bool, RepositoryError> {
        // The deliveries cascade; explicit anyway, so both engines read alike.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM webhook_deliveries WHERE hook_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let done = sqlx::query("DELETE FROM webhooks WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(done.rows_affected() > 0)
    }

    async fn list_webhook_deliveries(
        &self,
        hook_id: i64,
        limit: u32,
    ) -> Result<Vec<WebhookDelivery>, RepositoryError> {
        let rows: Vec<DeliveryRow<i32>> = sqlx::query_as(concat!(
            "SELECT ",
            delivery_columns!(),
            " FROM webhook_deliveries d \
             JOIN webhook_outbox e ON e.id = d.event_id \
             WHERE d.hook_id = $1 ORDER BY d.id DESC LIMIT $2"
        ))
        .bind(hook_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(delivery_from_row).collect()
    }

    async fn get_webhook_delivery(
        &self,
        delivery_id: i64,
    ) -> Result<Option<WebhookDelivery>, RepositoryError> {
        let row: Option<DeliveryRow<i32>> = sqlx::query_as(concat!(
            "SELECT ",
            delivery_columns!(),
            " FROM webhook_deliveries d \
             JOIN webhook_outbox e ON e.id = d.event_id WHERE d.id = $1"
        ))
        .bind(delivery_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        row.map(delivery_from_row).transpose()
    }

    async fn reset_webhook_delivery(
        &self,
        delivery_id: i64,
        now_iso: &str,
    ) -> Result<bool, RepositoryError> {
        // Not while a worker holds it: a leased row is `pending`/`failed`
        // with `next_attempt_at` in the future.
        let done = sqlx::query(
            "UPDATE webhook_deliveries SET status = 'pending', next_attempt_at = $1, \
             redelivered_at = $1, updated_at = $1 \
             WHERE id = $2 AND (status IN ('delivered', 'dead') OR next_attempt_at <= $1)",
        )
        .bind(now_iso)
        .bind(delivery_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(done.rows_affected() > 0)
    }

    async fn claim_due_webhook_deliveries(
        &self,
        now_iso: &str,
        lease_until_iso: &str,
        limit: u32,
    ) -> Result<Vec<DueWebhookDelivery>, RepositoryError> {
        // `FOR UPDATE SKIP LOCKED`: a row another replica's pass is claiming
        // at this moment is skipped, not waited for, so two passes never
        // send one delivery twice.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let ids: Vec<(i64,)> = sqlx::query_as(
            "UPDATE webhook_deliveries SET next_attempt_at = $1 WHERE id IN ( \
               SELECT d.id FROM webhook_deliveries d JOIN webhooks h ON h.id = d.hook_id \
               WHERE d.status IN ('pending', 'failed') AND d.next_attempt_at <= $2 AND NOT h.paused \
               ORDER BY d.next_attempt_at, d.id LIMIT $3 FOR UPDATE OF d SKIP LOCKED) \
             RETURNING id",
        )
        .bind(lease_until_iso)
        .bind(now_iso)
        .bind(limit as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let ids: Vec<i64> = ids.into_iter().map(|r| r.0).collect();
        let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(concat!(
            "SELECT ",
            delivery_columns!(),
            ", ",
            webhook_columns!(),
            ", e.producer, e.data_json \
             FROM webhook_deliveries d \
             JOIN webhook_outbox e ON e.id = d.event_id \
             JOIN webhooks h ON h.id = d.hook_id \
             WHERE d.id = ANY($1) ORDER BY d.next_attempt_at, d.id"
        ))
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let rows: Vec<DueRow> = rows.iter().map(due_row).collect::<Result<_, _>>()?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(|(delivery, hook, producer, data)| {
                Ok(DueWebhookDelivery {
                    delivery: delivery_from_row(delivery)?,
                    hook: webhook_from_row(hook)?,
                    producer,
                    data: data_from_json(&data)?,
                })
            })
            .collect()
    }

    async fn record_webhook_attempt(
        &self,
        delivery_id: i64,
        now_iso: &str,
        outcome: &WebhookAttemptOutcome,
        lease_until_iso: &str,
    ) -> Result<bool, RepositoryError> {
        let (status, attempted, next, code, error, delivered) = attempt_columns(outcome);
        // The lease guard: a row re-claimed by another pass carries that
        // pass's lease, and this (stale) outcome must not overwrite it.
        let done = sqlx::query(
            "UPDATE webhook_deliveries SET status = $1, attempts = attempts + $2, \
             next_attempt_at = COALESCE($3, next_attempt_at), last_response_code = $4, \
             last_error = $5, delivered_at = $6, updated_at = $7 \
             WHERE id = $8 AND next_attempt_at = $9",
        )
        .bind(status)
        .bind(if attempted { 1i32 } else { 0i32 })
        .bind(next)
        .bind(code.map(|c| c as i32))
        .bind(error)
        .bind(delivered.then_some(now_iso))
        .bind(now_iso)
        .bind(delivery_id)
        .bind(lease_until_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(done.rows_affected() > 0)
    }

    async fn count_pending_webhook_deliveries(&self) -> Result<u64, RepositoryError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM webhook_deliveries WHERE status IN ('pending', 'failed')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(n.max(0) as u64)
    }

    async fn delete_old_webhook_data(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let done = sqlx::query(
            "DELETE FROM webhook_deliveries WHERE status IN ('delivered', 'dead') AND updated_at < $1",
        )
        .bind(cutoff_iso)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "DELETE FROM webhook_outbox WHERE occurred_at < $1 \
             AND NOT EXISTS (SELECT 1 FROM webhook_deliveries d WHERE d.event_id = webhook_outbox.id)",
        )
        .bind(cutoff_iso)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(done.rows_affected())
    }
}

/// A claimed delivery's row, read by index: the delivery and event columns,
/// the hook columns, then the event's producer and data — too many columns
/// for sqlx's tuple decoding.
type DueRow = (DeliveryRow<i32>, WebhookRow, Option<String>, String);

fn due_row(row: &sqlx::postgres::PgRow) -> Result<DueRow, RepositoryError> {
    // A macro rather than a closure: each column decodes to its own type.
    macro_rules! col {
        ($i:expr) => {
            row.try_get($i)
                .map_err(|e| RepositoryError::Internal(e.to_string()))?
        };
    }
    Ok((
        (
            col!(0),
            col!(1),
            col!(2),
            col!(3),
            col!(4),
            col!(5),
            col!(6),
            col!(7),
            col!(8),
            col!(9),
            col!(10),
            col!(11),
            col!(12),
        ),
        (
            col!(13),
            col!(14),
            col!(15),
            col!(16),
            col!(17),
            col!(18),
            col!(19),
            col!(20),
            col!(21),
            col!(22),
            col!(23),
            col!(24),
            col!(25),
        ),
        col!(26),
        col!(27),
    ))
}

/// `(status, counts as an attempt, next_attempt_at, response code, error,
/// delivered)` — the columns one attempt outcome writes.
type AttemptColumns<'a> = (
    &'static str,
    bool,
    Option<&'a str>,
    Option<u16>,
    Option<&'a str>,
    bool,
);

fn attempt_columns(outcome: &WebhookAttemptOutcome) -> AttemptColumns<'_> {
    match outcome {
        WebhookAttemptOutcome::Delivered { code } => {
            ("delivered", true, None, Some(*code), None, true)
        }
        WebhookAttemptOutcome::Failed {
            code,
            error,
            next_attempt_at,
        } => (
            "failed",
            true,
            Some(next_attempt_at.as_str()),
            *code,
            Some(error.as_str()),
            false,
        ),
        WebhookAttemptOutcome::Dead { code, error } => {
            ("dead", true, None, *code, Some(error.as_str()), false)
        }
        WebhookAttemptOutcome::Expired { error } => {
            ("dead", false, None, None, Some(error.as_str()), false)
        }
    }
}

/// Insert outbox events and their fan-out rows inside the caller's
/// transaction — the one place a fact and its event are written together.
async fn write_outbox(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    events: &[NewWebhookEvent],
) -> Result<(), RepositoryError> {
    for event in events {
        sqlx::query(
            "INSERT INTO webhook_outbox (id, event_type, producer, occurred_at, data_json) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&event.id)
        .bind(event.event_type.as_str())
        .bind(event.producer.as_deref())
        .bind(&event.occurred_at)
        .bind(data_json(&event.data)?)
        .execute(&mut **tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for hook_id in &event.hook_ids {
            sqlx::query(
                "INSERT INTO webhook_deliveries (hook_id, event_id, status, attempts, next_attempt_at, updated_at) \
                 VALUES ($1, $2, 'pending', 0, $3, $4)",
            )
            .bind(hook_id)
            .bind(&event.id)
            .bind(&event.occurred_at)
            .bind(&event.occurred_at)
            .execute(&mut **tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
    }
    Ok(())
}
