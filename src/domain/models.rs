use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Bad Request: {0}")]
    BadRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Not Found: {0}")]
    NotFound(String),
    /// The pinned version exists and deliberately does not carry this endpoint —
    /// distinct from `NotFound`, which means the version itself is unknown.
    /// Maps to `410 Gone`.
    #[error("Gone: {0}")]
    Gone(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden")]
    Forbidden,
    /// A refusal whose remedy the caller should be told — e.g. a GA publish
    /// without the `releaser` role. Same `403` as [`AppError::Forbidden`],
    /// with a message.
    #[error("Forbidden: {0}")]
    ForbiddenWithReason(String),
    #[error("Internal Error: {0}")]
    Internal(String),
    #[error("Breaking Change: {0}")]
    BreakingChange(String),
    /// A Provide refused by the version rules (GA immutability, or a breaking
    /// change without a major bump). Answers `409` and carries the next free
    /// version the Producer should publish as instead.
    #[error("Version Conflict: {message}")]
    VersionConflict { message: String, proposed: SemVer },
}

impl From<crate::domain::ports::RepositoryError> for AppError {
    fn from(e: crate::domain::ports::RepositoryError) -> Self {
        match e {
            crate::domain::ports::RepositoryError::NotFound => {
                AppError::NotFound("Not found".to_string())
            }
            crate::domain::ports::RepositoryError::Conflict => {
                AppError::Conflict("Conflict".to_string())
            }
            crate::domain::ports::RepositoryError::Internal(msg) => AppError::Internal(msg),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiType {
    #[default]
    OpenApi,
    AsyncApi,
    Proto,
}

impl ApiType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiType::OpenApi => "openapi",
            ApiType::AsyncApi => "asyncapi",
            ApiType::Proto => "proto",
        }
    }
}

impl std::str::FromStr for ApiType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openapi" | "rest" => Ok(ApiType::OpenApi),
            "asyncapi" | "kafka" | "async" => Ok(ApiType::AsyncApi),
            "proto" | "grpc" => Ok(ApiType::Proto),
            _ => Err(format!("Unknown API type: {}", s)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Disabled,
    Dev,
    Local,
    Ldap,
}

impl AuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMode::Disabled => "disabled",
            AuthMode::Dev => "dev",
            AuthMode::Local => "local",
            AuthMode::Ldap => "ldap",
        }
    }
}

impl std::str::FromStr for AuthMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "disabled" | "off" | "maintenance" => Ok(AuthMode::Disabled),
            "dev" => Ok(AuthMode::Dev),
            "local" => Ok(AuthMode::Local),
            "ldap" => Ok(AuthMode::Ldap),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl serde::Serialize for SemVer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for SemVer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl SemVer {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn initial() -> Self {
        Self::new(1, 0, 0)
    }

    pub fn increment(&self, impact: Impact) -> Self {
        match impact {
            Impact::Major => Self::new(self.major + 1, 0, 0),
            Impact::Minor => Self::new(self.major, self.minor + 1, 0),
            Impact::Patch => Self::new(self.major, self.minor, self.patch + 1),
            Impact::None => *self,
        }
    }

    /// Parse a Producer-declared spec version: `MAJOR[.MINOR[.PATCH]]` with an
    /// optional leading `v`; omitted parts are zero, so `v2` and `2.0` both
    /// mean `2.0.0`. The rejection people will actually hit gets a specific
    /// message: a `-SNAPSHOT` (or any suffix) — stability is declared on the
    /// Provide, never encoded in the version string.
    pub fn parse_spec_version(raw: &str) -> Result<SemVer, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(
                "version is empty — expected MAJOR[.MINOR[.PATCH]] (e.g. 1.2.0)".to_string(),
            );
        }
        if trimmed.to_uppercase().contains("-SNAPSHOT") {
            return Err(format!(
                "invalid version '{}': snapshot status is declared by the Provide's stability flag, never encoded in the version string — publish '{}' with stability=snapshot instead",
                trimmed,
                trimmed
                    .to_uppercase()
                    .replace("-SNAPSHOT", "")
                    .to_lowercase()
            ));
        }
        if trimmed.contains('-') || trimmed.contains('+') {
            return Err(format!(
                "invalid version '{}': pre-release suffixes and build metadata are not accepted — expected MAJOR[.MINOR[.PATCH]] (e.g. 1.2.0)",
                trimmed
            ));
        }
        trimmed.parse::<SemVer>().map_err(|_| {
            format!(
                "invalid version '{}': expected MAJOR[.MINOR[.PATCH]] with numeric parts, optionally 'v'-prefixed (e.g. 1.2.0, v2.1, 2)",
                trimmed
            )
        })
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl std::str::FromStr for SemVer {
    type Err = String;
    /// Accepts `MAJOR[.MINOR[.PATCH]]` with an optional leading `v`/`V`;
    /// omitted parts are zero (`v2` → `2.0.0`). The canonical form
    /// ([`Display`](std::fmt::Display)) is always the full three-part
    /// `MAJOR.MINOR.PATCH`, so lenient input never leaks into storage or
    /// responses.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let parts: Vec<&str> = digits.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return Err(format!("Invalid SemVer: {}", s));
        }
        let mut numbers = [0u32; 3];
        for (slot, part) in numbers.iter_mut().zip(&parts) {
            *slot = part
                .parse()
                .map_err(|e| format!("Invalid SemVer '{}': {}", s, e))?;
        }
        Ok(SemVer {
            major: numbers[0],
            minor: numbers[1],
            patch: numbers[2],
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum Impact {
    #[default]
    None,
    Patch,
    Minor,
    Major,
}

/// Which of exactly two states a stored Version is in (see `CONTEXT.md`).
///
/// Declared by the caller on every Provide; a state of the stored version, not
/// part of the version string. `Snapshot` rows are overwritable and may
/// expire; `Ga` rows are immutable and permanently claim their number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stability {
    Snapshot,
    Ga,
}

impl Stability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stability::Snapshot => "snapshot",
            Stability::Ga => "ga",
        }
    }
}

impl std::str::FromStr for Stability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "snapshot" => Ok(Stability::Snapshot),
            "ga" => Ok(Stability::Ga),
            _ => Err(format!(
                "Unknown stability: {} (expected 'snapshot' or 'ga')",
                s
            )),
        }
    }
}

/// One entry of a version line: a Producer's spec for one API type under one
/// producer-declared version, without the stored document body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecVersionMeta {
    pub id: i64,
    pub service_id: i64,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub content_hash: String,
    /// The authenticated Actor credited with this version's content: the last
    /// provider — preserved across a same-content promotion, so the human who
    /// built the snapshot stays on the released version.
    pub provided_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_required_at: Option<String>,
    /// When trunk CI last provided this entry (ADR-0004); `None` = never.
    pub trunk_provided_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProvideResponse {
    pub version: SemVer,
    pub stability: Stability,
    pub content_hash: String,
    pub changes: ProvideChanges,
    /// True when this Provide released an existing snapshot in place — a state
    /// change listeners care about even when the content is byte-identical.
    #[serde(default)]
    pub promoted: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProvideChanges {
    pub inserts: usize,
    pub updates: usize,
    pub deletes: usize,
}

impl ProvideChanges {
    /// True when a provide produced no endpoint changes (a no-op re-upload).
    pub fn is_empty(&self) -> bool {
        self.inserts == 0 && self.updates == 0 && self.deletes == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LdapConfig {
    pub server_url: String,
    pub bind_dn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_password: Option<String>,
    pub base_dn: String,
    #[serde(default = "LdapConfig::default_user_filter")]
    pub user_filter: String,
    #[serde(default)]
    pub group_filter: String,
    #[serde(default)]
    pub admin_group: String,
    #[serde(default)]
    pub use_tls: bool,
}

impl LdapConfig {
    fn default_user_filter() -> String {
        "(uid={username})".to_string()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.server_url.is_empty() {
            return Err("Server URL must not be empty".to_string());
        }
        if !self.server_url.starts_with("ldap://") && !self.server_url.starts_with("ldaps://") {
            return Err("Server URL must start with ldap:// or ldaps://".to_string());
        }
        let host_part = if self.server_url.starts_with("ldap://") {
            &self.server_url[7..]
        } else {
            &self.server_url[8..]
        };
        if host_part.is_empty() || host_part.contains(' ') || host_part.starts_with('/') {
            return Err("Invalid LDAP server host".to_string());
        }
        if self.bind_dn.is_empty() {
            return Err("Bind DN must not be empty".to_string());
        }
        if self.base_dn.is_empty() {
            return Err("Base DN must not be empty".to_string());
        }
        Ok(())
    }
}

/// Represents an authenticated user from any auth provider.
///
/// Carries identity only. What the user may do is resolved separately, from
/// role grants and directory group membership — an auth provider's job ends at
/// establishing who somebody is.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    pub username: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing, default)]
    pub password_hash: String,
    pub approved: bool,
}

/// Where a Group's membership comes from.
///
/// The distinction is not cosmetic: a `Native` group's membership is Sanshain's
/// to edit, while an `Ldap` group's belongs to the directory and is resolved
/// when a caller is authorised rather than stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupSource {
    Native,
    Ldap,
}

impl GroupSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupSource::Native => "native",
            GroupSource::Ldap => "ldap",
        }
    }

    pub fn parse(value: &str) -> Option<GroupSource> {
        match value {
            "native" => Some(GroupSource::Native),
            "ldap" => Some(GroupSource::Ldap),
            _ => None,
        }
    }
}

/// A set of users that roles attach to.
///
/// Two groups may share a name provided they differ in source, so a directory
/// group and a Sanshain group called the same thing stay distinct entities.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub source: GroupSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub user_id: i64,
    pub expires_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EndpointRecord {
    pub id: Option<i64>,
    pub api_type: ApiType,
    pub path: String,
    pub normalized_path: String,
    pub method: String,
    pub yaml_content: String,
    #[serde(default)]
    pub deprecated: bool,
}

/// How a request for an endpoint at a Pin was answered.
///
/// See `CONTEXT.md` and `docs/adr/0003-versions-replace-branches.md`. The
/// states are exhaustive: every resolution is exactly one of them, and every
/// answer names the stability it was served from. There is no fallback beyond
/// GA-before-Snapshot for the pinned number, and nothing waits: all failures
/// are immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolutionState {
    /// The pinned version exists and contains the endpoint.
    Served,
    /// The pinned version exists and did not include this endpoint, so it is
    /// deliberately not part of that version's API. A definitive "no" — maps
    /// to `410 Gone`.
    Absent,
    /// The Producer's line has no such version in either stability. A
    /// configuration error on the Consumer's side — maps to `404`.
    Unknown,
}

impl ResolutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolutionState::Served => "served",
            ResolutionState::Absent => "absent",
            ResolutionState::Unknown => "unknown",
        }
    }

    /// True when no endpoint was produced. `Absent` and `Unknown` both mean
    /// "nothing to serve", but they are *not* interchangeable: `Absent` is a
    /// deliberate omission from an existing version, `Unknown` a version that
    /// does not exist.
    pub fn is_empty(&self) -> bool {
        matches!(self, ResolutionState::Absent | ResolutionState::Unknown)
    }
}

/// The endpoint a resolution produced, when it produced one.
#[derive(Clone, Debug)]
pub struct ResolvedEndpoint {
    pub id: i64,
    pub yaml_content: String,
    pub deprecated: bool,
}

/// The outcome of resolving one endpoint against one Pin.
#[derive(Clone, Debug)]
pub struct EndpointResolution {
    pub state: ResolutionState,
    /// The stability the endpoint was actually served from. Set for `Served`;
    /// `None` when nothing was served.
    pub served_stability: Option<Stability>,
    pub endpoint: Option<ResolvedEndpoint>,
}

impl EndpointResolution {
    pub fn served(stability: Stability, endpoint: ResolvedEndpoint) -> Self {
        Self {
            state: ResolutionState::Served,
            served_stability: Some(stability),
            endpoint: Some(endpoint),
        }
    }

    pub fn absent() -> Self {
        Self {
            state: ResolutionState::Absent,
            served_stability: None,
            endpoint: None,
        }
    }

    pub fn unknown() -> Self {
        Self {
            state: ResolutionState::Unknown,
            served_stability: None,
            endpoint: None,
        }
    }
}

/// A message-level AsyncAPI channel contract (ai/improvements.md item #20).
///
/// Kafka topic names share a global namespace, so contract identity is the
/// message -- keyed by `(channel, message_name)` -- not the whole channel
/// payload. Only `publish` (PUB) messages register a contract; the first
/// providing service becomes the `owner` and is the only one allowed to widen
/// the message schema. A different service may co-publish the same
/// `(channel, message_name)` only if its payload schema is identical.
/// Registered and enforced on GA provides only — snapshots are never
/// compat-checked.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelMessageContract {
    pub channel: String,
    pub message_name: String,
    pub owner_service_id: i64,
    pub payload_yaml: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyReport {
    pub unused_endpoints: Vec<EndpointInfo>,
    pub missing_endpoints: Vec<MissingEndpointInfo>,
    pub dependency_graph: Vec<DependencyInfo>,
    #[serde(default)]
    pub service_tags: HashMap<String, Vec<String>>,
    /// The current trunk pin set (ADR-0004) — the main graph's edges.
    /// Populated by the application layer; empty in raw repository results.
    #[serde(default)]
    pub trunk_graph: Vec<TrunkPinInfo>,
    /// Trunk entries not refreshed since this instant are stale (half the
    /// trunk TTL). `None` when the TTL is disabled. Application-populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_stale_before: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct EndpointInfo {
    pub api_type: ApiType,
    pub service: String,
    pub version: SemVer,
    pub stability: Stability,
    pub path: String,
    pub method: String,
    #[serde(default)]
    pub deprecated: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct MissingEndpointInfo {
    pub api_type: ApiType,
    pub client: String,
    pub service: String,
    pub version: SemVer,
    pub path: String,
    pub method: String,
}

/// A sanshain-branch (ADR-0005): a named graph created by a releaser as a
/// copy of a source graph at a chosen instant, updated by tagged builds.
/// Identity is the id; the unique-among-live name is a rename-safe label.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BranchInfo {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub created_by: String,
    /// Provenance: `trunk`, or the source branch's then-current name.
    pub source: String,
    /// The instant of the source graph the branch was born from.
    pub as_of: String,
}

/// One current trunk pin (ADR-0004): the open record of the append-only trunk
/// dependency store, joined to names. The version is by value — it may
/// reference a deleted entry (dangling until re-provided).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrunkPinInfo {
    pub client: String,
    pub service: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub path: String,
    pub method: String,
    /// When this pin became the current one.
    pub valid_from: String,
    /// Refreshed by every identical trunk re-require.
    pub last_required_at: String,
    /// The pinned version no longer exists (deleted entry) — a visibly
    /// dangling by-value reference that heals when the number is re-provided.
    /// Computed on read; serialized only when true.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dangling: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyInfo {
    pub api_type: ApiType,
    pub client: String,
    pub service: String,
    /// The Consumer's Pin.
    pub version: SemVer,
    /// The pinned version's current stability.
    pub stability: Stability,
    pub path: String,
    pub method: String,
    #[serde(default)]
    pub deprecated: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConsumerEndpointInfo {
    pub api_type: ApiType,
    pub service: String,
    pub version: SemVer,
    pub stability: Stability,
    pub path: String,
    pub method: String,
    pub yaml_content: Option<String>,
    #[serde(default)]
    pub deprecated: bool,
}

/// One version-line entry as listed on a Producer, UI- and wire-facing.
#[derive(Serialize, Clone, Debug)]
pub struct ProducerVersionInfo {
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub content_hash: String,
    pub provided_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_required_at: Option<String>,
    pub endpoint_count: i64,
    /// Use-based expiry for snapshots when cleanup is enabled; `None` for GA
    /// (never age-culled) or when cleanup is off. Populated by the application
    /// layer; `None` in raw repository results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// When trunk CI last provided this entry (ADR-0004); `None` = never.
    pub trunk_provided_at: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProducerSummary {
    pub name: String,
    /// Every entry of the Producer's version lines, all API types. Populated
    /// by the application layer; empty in raw repository results.
    #[serde(default)]
    pub versions: Vec<ProducerVersionInfo>,
    pub is_favorite: bool,
    pub icon: Option<String>,
    pub domain: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserFavoritesResponse {
    pub services: Vec<String>,
    pub clients: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    /// Service name, when the emitting event attached a `service` field
    /// (e.g. a provide/require log line). `None` for events with no such context.
    pub service: Option<String>,
    /// Version string, when the emitting event attached a `version` field.
    pub version: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct LogResponse {
    pub errors: Vec<LogEntry>,
    pub warnings: Vec<LogEntry>,
    pub infos: Vec<LogEntry>,
    pub debugs: Vec<LogEntry>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct SystemStats {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub system_uptime: u64,
    pub process_uptime: u64,
    pub requests_total: u64,
    pub failures_total: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DebugConfig {
    pub business_logic_debug: bool,
    pub admin_user_debug: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct CacheStats {
    pub enabled: bool,
    pub memory_limit_mb: u64,
    pub estimated_memory_used_bytes: u64,
    pub entry_count: u64,
    pub hit_count: u64,
    pub miss_count: u64,
    pub hit_rate_percent: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub timestamp: String,
    pub username: String,
    pub action: String,
    pub details: String,
    pub service: Option<String>,
    /// Version string for 2.0 entries. Branch-era audit entries survive in the
    /// same column and stay readable — they just describe a world that no
    /// longer exists.
    pub version: Option<String>,
    pub action_type: Option<String>,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditLogFilter {
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub action_type: Option<String>,
    pub service_wildcard: Option<String>,
    pub version_wildcard: Option<String>,
    pub limit: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Lenient input, canonical storage: short and `v`-prefixed forms fill
    /// omitted parts with zeroes and always display as full three-part.
    #[test]
    fn semver_accepts_short_and_v_prefixed_forms_with_implicit_zeroes() {
        for (input, expected) in [
            ("2", SemVer::new(2, 0, 0)),
            ("1.2", SemVer::new(1, 2, 0)),
            ("1.2.3", SemVer::new(1, 2, 3)),
            ("v2", SemVer::new(2, 0, 0)),
            ("v1.2", SemVer::new(1, 2, 0)),
            ("V1.2.3", SemVer::new(1, 2, 3)),
        ] {
            let parsed = SemVer::from_str(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(parsed, expected, "{input}");
        }
        assert_eq!(SemVer::from_str("v1.2").unwrap().to_string(), "1.2.0");
    }

    #[test]
    fn semver_rejects_malformed_versions() {
        for input in [
            "",
            "v",
            "1.",
            "1..2",
            "1.2.3.4",
            "one",
            "v1.x",
            "1.2.3-SNAPSHOT",
        ] {
            assert!(
                SemVer::from_str(input).is_err(),
                "'{input}' must be rejected"
            );
        }
    }

    /// The spec-version wrapper accepts the lenient forms and keeps its
    /// specific guidance for suffixed versions.
    #[test]
    fn parse_spec_version_accepts_lenient_forms_and_keeps_suffix_guidance() {
        assert_eq!(
            SemVer::parse_spec_version(" v2.1 ").unwrap(),
            SemVer::new(2, 1, 0)
        );
        let err = SemVer::parse_spec_version("1.2.0-SNAPSHOT").unwrap_err();
        assert!(err.contains("stability"), "{err}");
        let err = SemVer::parse_spec_version("1.2.beta").unwrap_err();
        assert!(err.contains("MAJOR[.MINOR[.PATCH]]"), "{err}");
    }

    #[test]
    fn provide_changes_is_empty_only_when_all_zero() {
        assert!(ProvideChanges::default().is_empty());
        assert!(
            ProvideChanges {
                inserts: 0,
                updates: 0,
                deletes: 0
            }
            .is_empty()
        );
        assert!(
            !ProvideChanges {
                inserts: 1,
                updates: 0,
                deletes: 0
            }
            .is_empty()
        );
        assert!(
            !ProvideChanges {
                inserts: 0,
                updates: 0,
                deletes: 3
            }
            .is_empty()
        );
    }

    #[test]
    fn api_type_accepts_canonical_names_and_legacy_aliases() {
        let cases = [
            ("openapi", ApiType::OpenApi),
            ("REST", ApiType::OpenApi),
            ("asyncapi", ApiType::AsyncApi),
            ("kafka", ApiType::AsyncApi),
            ("async", ApiType::AsyncApi),
            ("proto", ApiType::Proto),
            ("grpc", ApiType::Proto),
        ];

        for (input, expected) in cases {
            assert_eq!(ApiType::from_str(input).unwrap(), expected);
        }
    }

    #[test]
    fn api_type_rejects_unknown_values() {
        assert_eq!(
            ApiType::from_str("soap").unwrap_err(),
            "Unknown API type: soap"
        );
    }

    #[test]
    fn api_type_as_str_returns_stable_wire_values() {
        assert_eq!(ApiType::OpenApi.as_str(), "openapi");
        assert_eq!(ApiType::AsyncApi.as_str(), "asyncapi");
        assert_eq!(ApiType::Proto.as_str(), "proto");
    }

    #[test]
    fn auth_mode_parses_and_formats_supported_modes() {
        let cases = [
            ("dev", AuthMode::Dev),
            ("local", AuthMode::Local),
            ("ldap", AuthMode::Ldap),
        ];

        for (input, expected) in cases {
            let parsed = AuthMode::from_str(input).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), input);
        }
        assert!(AuthMode::from_str("oauth").is_err());
    }

    #[test]
    fn app_error_maps_repository_errors_at_the_boundary() {
        let not_found: AppError = crate::domain::ports::RepositoryError::NotFound.into();
        let conflict: AppError = crate::domain::ports::RepositoryError::Conflict.into();
        let internal: AppError =
            crate::domain::ports::RepositoryError::Internal("boom".into()).into();

        assert!(matches!(not_found, AppError::NotFound(message) if message == "Not found"));
        assert!(matches!(conflict, AppError::Conflict(message) if message == "Conflict"));
        assert!(matches!(internal, AppError::Internal(message) if message == "boom"));
    }

    #[test]
    fn ldap_config_defaults_to_uid_user_filter() {
        let config: LdapConfig = serde_json::from_value(serde_json::json!({
            "server_url": "ldap://example.test",
            "bind_dn": "cn=admin,dc=example,dc=test",
            "base_dn": "dc=example,dc=test"
        }))
        .unwrap();

        assert_eq!(config.user_filter, "(uid={username})");
        assert!(config.group_filter.is_empty());
        assert!(config.admin_group.is_empty());
        assert!(!config.use_tls);
    }

    #[test]
    fn ldap_config_serializes_password_for_persistence() {
        let serialized = serde_json::to_value(LdapConfig {
            server_url: "ldap://example.test".into(),
            bind_dn: "cn=admin,dc=example,dc=test".into(),
            bind_password: Some("secret".into()),
            base_dn: "dc=example,dc=test".into(),
            user_filter: "(uid={username})".into(),
            group_filter: String::new(),
            admin_group: String::new(),
            use_tls: false,
        })
        .unwrap();

        assert_eq!(serialized["bind_password"], "secret");
    }

    #[test]
    fn ldap_config_validation_accepts_ldap_and_ldaps_hosts() {
        for server_url in ["ldap://example.test", "ldaps://example.test"] {
            let config = LdapConfig {
                server_url: server_url.into(),
                bind_dn: "cn=admin,dc=example,dc=test".into(),
                bind_password: None,
                base_dn: "dc=example,dc=test".into(),
                user_filter: "(uid={username})".into(),
                group_filter: String::new(),
                admin_group: String::new(),
                use_tls: false,
            };

            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn ldap_config_validation_rejects_invalid_required_fields() {
        let valid = LdapConfig {
            server_url: "ldap://example.test".into(),
            bind_dn: "cn=admin,dc=example,dc=test".into(),
            bind_password: None,
            base_dn: "dc=example,dc=test".into(),
            user_filter: "(uid={username})".into(),
            group_filter: String::new(),
            admin_group: String::new(),
            use_tls: false,
        };

        let mut config = valid.clone();
        config.server_url.clear();
        assert_eq!(
            config.validate().unwrap_err(),
            "Server URL must not be empty"
        );

        let mut config = valid.clone();
        config.server_url = "https://example.test".into();
        assert_eq!(
            config.validate().unwrap_err(),
            "Server URL must start with ldap:// or ldaps://"
        );

        let mut config = valid.clone();
        config.server_url = "ldap://bad host".into();
        assert_eq!(config.validate().unwrap_err(), "Invalid LDAP server host");

        let mut config = valid.clone();
        config.server_url = "ldap:///bad".into();
        assert_eq!(config.validate().unwrap_err(), "Invalid LDAP server host");

        let mut config = valid.clone();
        config.bind_dn.clear();
        assert_eq!(config.validate().unwrap_err(), "Bind DN must not be empty");

        let mut config = valid;
        config.base_dn.clear();
        assert_eq!(config.validate().unwrap_err(), "Base DN must not be empty");
    }
}
