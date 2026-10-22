use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
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

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
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
    /// Human login via an external OpenID Connect provider (browser redirect
    /// flow). Machine/Consumer clients keep API tokens — OIDC is human-only.
    Oidc,
}

impl AuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMode::Disabled => "disabled",
            AuthMode::Dev => "dev",
            AuthMode::Local => "local",
            AuthMode::Ldap => "ldap",
            AuthMode::Oidc => "oidc",
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
            "oidc" => Ok(AuthMode::Oidc),
            _ => Err(()),
        }
    }
}

/// OpenID Connect provider configuration for human browser login (stored in
/// settings as JSON, like [`LdapConfig`]). The client secret is confidential;
/// it is never serialized back out to the admin UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OidcConfig {
    /// The provider's issuer URL — its discovery document is at
    /// `<issuer_url>/.well-known/openid-configuration`.
    pub issuer_url: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Where the provider redirects back to — must be this service's
    /// `/auth/oidc/callback`, and registered with the provider.
    pub redirect_url: String,
    #[serde(default = "OidcConfig::default_scopes")]
    pub scopes: Vec<String>,
    /// ID-token claim carrying the username (default `preferred_username`).
    #[serde(default = "OidcConfig::default_username_claim")]
    pub username_claim: String,
    /// ID-token claim carrying the user's groups (default `groups`).
    #[serde(default = "OidcConfig::default_groups_claim")]
    pub groups_claim: String,
    /// Group whose members are administrators, if any.
    #[serde(default)]
    pub admin_group: String,
}

impl OidcConfig {
    fn default_scopes() -> Vec<String> {
        vec!["openid".into(), "email".into(), "profile".into()]
    }
    fn default_username_claim() -> String {
        "preferred_username".to_string()
    }
    fn default_groups_claim() -> String {
        "groups".to_string()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.issuer_url.starts_with("https://") && !self.issuer_url.starts_with("http://") {
            return Err("Issuer URL must start with http:// or https://".to_string());
        }
        if self.client_id.is_empty() {
            return Err("Client ID must not be empty".to_string());
        }
        if !self.redirect_url.contains("/auth/oidc/callback") {
            return Err("Redirect URL must point at /auth/oidc/callback".to_string());
        }
        Ok(())
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
    /// AsyncAPI subscribe operations harvested from this provide as consumer
    /// edges (ai/improvements.md #6) — each with its resolved owner (or none)
    /// and, on a snapshot, an advisory drift note. Empty for non-AsyncAPI
    /// provides. Omitted from the JSON when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub harvested_subscriptions: Vec<HarvestedSubscriptionResult>,
    /// The stream this provide was recorded in, and the warnings when the
    /// declared one could not be honoured. Not part of the body: the stream is
    /// for the audit trail, the warnings travel as `Sanshain-Warning` headers.
    #[serde(skip)]
    pub stream: ResolvedStream,
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
    /// The server URL must be `ldap://host[:port]` or `ldaps://host[:port]`.
    /// A typo such as `ldap:/host:636` (one slash) parses as a URL without a
    /// host and fails later as an unexplained "connection refused"; it is
    /// refused where it is typed instead, naming the expected form.
    pub fn validate_server_url(&self) -> Result<(), String> {
        let url = self.server_url.trim();
        let expected =
            "expected ldap://host:port or ldaps://host:port (e.g. ldaps://dc01.example.com:636)";
        let rest = url
            .strip_prefix("ldaps://")
            .or_else(|| url.strip_prefix("ldap://"))
            .ok_or_else(|| {
                format!("LDAP server URL '{url}' has no ldap:// or ldaps:// scheme — {expected}")
            })?;
        let authority = rest.split('/').next().unwrap_or_default();
        // `[::1]:636` keeps its colons inside the brackets.
        let (host, port) = match authority.strip_prefix('[') {
            Some(bracketed) => match bracketed.split_once(']') {
                Some((host, tail)) => (host, tail.strip_prefix(':')),
                None => ("", None),
            },
            None => match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            },
        };
        if host.is_empty() {
            return Err(format!(
                "LDAP server URL '{url}' names no host — {expected}"
            ));
        }
        if let Some(port) = port
            && !matches!(port.parse::<u16>(), Ok(p) if p > 0)
        {
            return Err(format!(
                "LDAP server URL '{url}' has no valid port — {expected}"
            ));
        }
        Ok(())
    }

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

/// What an API token may do.
///
/// Decided at creation and immutable afterwards: a token handed to an AI or
/// MCP client is minted read-only and stays so, whatever the client asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiTokenScope {
    /// Everything the owning user may do through the API.
    #[default]
    Full,
    /// Reading only: reports, versions, update streams and the MCP endpoint.
    /// Every state-changing API route refuses it.
    ReadOnly,
}

impl ApiTokenScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiTokenScope::Full => "full",
            ApiTokenScope::ReadOnly => "read_only",
        }
    }

    /// The stored spelling back to the scope; anything else is `None` rather
    /// than a silent default, so a corrupt row cannot widen a token.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "full" => Some(ApiTokenScope::Full),
            "read_only" => Some(ApiTokenScope::ReadOnly),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    /// The SHA-256 of the raw token, compared on every request. Never
    /// serialized: it is a server-side secret, and the token listing is what
    /// the account page receives.
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub scope: ApiTokenScope,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
    /// The administrator who issued the token for a service account; none
    /// for a token its owner created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_by: Option<String>,
}

/// The caller behind a validated API token: who it belongs to and what it may
/// do. `last_used_at` is carried so the application layer can decide whether
/// the stamp is due without a second read.
#[derive(Clone, Debug)]
pub struct ApiTokenIdentity {
    pub token_id: String,
    pub scope: ApiTokenScope,
    pub last_used_at: Option<String>,
    pub user: User,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing, default)]
    pub password_hash: String,
    pub approved: bool,
}

/// The password hash of a service account: not a hash at all, so no password
/// ever matches it, the way `!` locks an account in `/etc/shadow`.
pub const SERVICE_ACCOUNT_PASSWORD_HASH: &str = "!service-account";

impl User {
    /// An account an administrator created for automation: it never signs
    /// in, and administrators issue its API tokens.
    pub fn is_service_account(&self) -> bool {
        self.password_hash == SERVICE_ACCOUNT_PASSWORD_HASH
    }
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

/// What `find_endpoint` looks for: a case-insensitive substring of an
/// endpoint's path, channel or method name, `*` matching anything, narrowed
/// by method and API type, in the latest version of every line unless
/// `all_versions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSearch {
    pub query: String,
    pub method: Option<String>,
    pub api_type: Option<ApiType>,
    pub all_versions: bool,
}

/// One endpoint found by `find_endpoint`, with how many distinct Consumers
/// pin it at that version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EndpointSearchHit {
    pub producer: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub method: String,
    pub path: String,
    pub deprecated: bool,
    pub consumer_count: i64,
    /// The one-line description derived from the snippet by the application;
    /// repositories leave it empty.
    pub summary: Option<String>,
    /// The endpoint's own snippet, for deriving `summary`; never serialized.
    #[serde(skip)]
    pub yaml_content: String,
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

/// A resolved SUB to reconcile into the harvested store (ai/improvements.md #6,
/// ADR-0006): the subscribed `(channel, message_name)` and the PUB owner it
/// resolved to against the GA contract store — `None` when no GA provider owns
/// the channel yet (an unfulfilled subscription, kept visible).
#[derive(Clone, Debug, PartialEq)]
pub struct HarvestedSubInput {
    pub channel: String,
    pub message_name: String,
    pub owner_service_id: Option<i64>,
    pub owner_name: Option<String>,
}

/// A current (open) harvested subscription edge, for the graph/report. A
/// **version-less** consumer edge (ADR-0006): a subscription pins no producer
/// version. `owner` is `None` for an unfulfilled subscription.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HarvestedSubscription {
    pub client: String,
    pub channel: String,
    pub message_name: String,
    pub owner: Option<String>,
    pub trunk: bool,
}

/// One edge from a system to a **Peripheral Resource** (#46): something outside
/// Sanshain the system depends on, known only by its Backstage entity ref
/// (`resource:<namespace>/<name>`, lower-cased). The ref is the node's
/// identity, so systems sharing a resource point at one node.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeripheralDependency {
    pub system: String,
    pub resource: String,
}

/// The origin of a version: what the build that provided it said about why
/// the bytes are what they are. References, not content: the build that
/// produced it, the change (pull or merge request) behind it, the tickets it
/// closes and one line of intent. Every field optional; the latest Provide
/// that carries any of them wins.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct VersionOrigin {
    pub build_url: Option<String>,
    pub change_url: Option<String>,
    #[serde(default)]
    pub tickets: Vec<String>,
    pub note: Option<String>,
}

impl VersionOrigin {
    /// At most this many ticket ids are kept.
    pub const MAX_TICKETS: usize = 20;
    /// A ticket id is at most this long.
    pub const MAX_TICKET_CHARS: usize = 64;
    /// The note is one line of at most this many characters.
    pub const MAX_NOTE_CHARS: usize = 200;
    /// A URL longer than this is refused.
    pub const MAX_URL_CHARS: usize = 2048;

    pub fn is_empty(&self) -> bool {
        self.build_url.is_none()
            && self.change_url.is_none()
            && self.tickets.is_empty()
            && self.note.is_none()
    }

    /// The origin as stored: URLs absolute `http(s)`, ticket ids trimmed,
    /// deduplicated and of letters, digits, `-`, `_` and `.`, the note
    /// trimmed with its whitespace collapsed. Blank text is absent. A value
    /// outside those bounds is refused by name so a misconfigured pipeline
    /// learns it at once.
    pub fn normalized(&self) -> Result<VersionOrigin, String> {
        let url = |field: &str, value: &Option<String>| -> Result<Option<String>, String> {
            let Some(raw) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
                return Ok(None);
            };
            if raw.chars().count() > Self::MAX_URL_CHARS {
                return Err(format!(
                    "{field} is longer than {} characters",
                    Self::MAX_URL_CHARS
                ));
            }
            let ok = (raw.starts_with("https://") || raw.starts_with("http://"))
                && raw.len() > 8
                && !raw.chars().any(|c| c.is_whitespace() || c.is_control());
            if !ok {
                return Err(format!("{field} must be an absolute http(s) URL"));
            }
            Ok(Some(raw.to_string()))
        };
        let mut tickets: Vec<String> = Vec::new();
        for raw in &self.tickets {
            let ticket = raw.trim();
            if ticket.is_empty() {
                continue;
            }
            let valid = ticket.chars().count() <= Self::MAX_TICKET_CHARS
                && ticket
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if !valid {
                return Err(format!(
                    "ticket '{ticket}' is not an id of letters, digits, '-', '_' and '.' (at most {} characters)",
                    Self::MAX_TICKET_CHARS
                ));
            }
            if !tickets.iter().any(|t| t == ticket) {
                tickets.push(ticket.to_string());
            }
        }
        if tickets.len() > Self::MAX_TICKETS {
            return Err(format!("at most {} tickets", Self::MAX_TICKETS));
        }
        let note = self
            .note
            .as_deref()
            .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|n| !n.is_empty());
        if let Some(n) = &note
            && n.chars().count() > Self::MAX_NOTE_CHARS
        {
            return Err(format!(
                "note is longer than {} characters",
                Self::MAX_NOTE_CHARS
            ));
        }
        Ok(VersionOrigin {
            build_url: url("build_url", &self.build_url)?,
            change_url: url("change_url", &self.change_url)?,
            tickets,
            note,
        })
    }
}

/// Who owns a system per its Backstage `spec.owner` (#48): the ref's kind
/// (`group` unless the ref says otherwise, Backstage's default for owners)
/// and name, both lower-cased like every other ref. The namespace is not
/// kept — the owner is shown, never resolved.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SystemOwner {
    pub kind: String,
    pub name: String,
}

/// One of a system's Backstage `metadata.links` (#48). `url` is always
/// `http` or `https`; `title` defaults to the URL's host; `type` and `icon`
/// are free text passed through for the UI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SystemLink {
    pub url: String,
    pub title: String,
    #[serde(rename = "type")]
    pub link_type: Option<String>,
    pub icon: Option<String>,
}

/// What a system's `catalog-info.yaml` says about it beyond its periphery
/// (#48): owner, description, where its code lives and its links. A
/// present-day fact about the system, not a graph fact: one per system,
/// replaced on every push, never tracked per stream or branch. Every field
/// may be absent — a partial catalog-info records what it has.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct SystemContext {
    pub owner: Option<SystemOwner>,
    pub description: Option<String>,
    /// The `backstage.io/source-location` annotation without its `url:`
    /// prefix; only kept when it is an `http` or `https` URL.
    pub source_location: Option<String>,
    pub links: Vec<SystemLink>,
    /// `metadata.tags` (ADR-0010), lower-cased, deduplicated, at most 20.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A Producer's or Consumer's context answer (#48): the name as asked for,
/// and its context — `None` when no catalog push described it yet.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ParticipantContext {
    pub name: String,
    pub context: Option<SystemContext>,
}

/// Which periphery set a read or a write addresses (#46). A trunk push
/// updates both `Trunk` and `Dev` — the dev view is all activity, trunk
/// included; a sanshain-branch gets its set only as a copy at the cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeripheryTrack {
    Dev,
    Trunk,
    Branch(i64),
}

/// Which member-version store a read addresses: trunk's (every trunk-flagged
/// Provide) or one sanshain-branch's (tag-flagged Provides, ADR-0005).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberTrack {
    Trunk,
    Branch(i64),
}

/// One member-version record, as it was current at the read's instant: the
/// version of a producer's line that trunk or a branch carries. The producer
/// is by value (it may have been deleted since). `last_provided_at` is the
/// latest trunk Provide of that version (trunk only; `None` for branches).
#[derive(Clone, Debug, PartialEq)]
pub struct MemberVersion {
    pub service: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub valid_from: String,
    pub last_provided_at: Option<String>,
}

/// One version line of a producer in a graph scope at the scope's instant.
/// `stability` is `None` when the version was deleted since (a dangling
/// reference); `last_provided_at` is `None` when no provide at or before the
/// instant is known.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ScopeVersionLine {
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Option<Stability>,
    pub last_provided_at: Option<String>,
}

/// A producer in a graph scope at the scope's instant, with its version lines
/// there — one per API type.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ScopeProducer {
    pub name: String,
    pub versions: Vec<ScopeVersionLine>,
}

/// What the graph needs of a scope beyond its pin set: the producers in it
/// with their version lines, and the harvested subscriptions that belong to
/// it (dev = all, live main = trunk-flagged, anything else = none).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GraphScopeData {
    pub producers: Vec<ScopeProducer>,
    pub harvested_subscriptions: Vec<HarvestedSubscription>,
    /// The context (#48) of every system that pushed one, by name, so the
    /// node panel needs no extra request. Not scoped: a present-day fact.
    pub systems: BTreeMap<String, SystemContext>,
}

/// The answer to a catalog push (#46): the periphery now recorded for the
/// system, so the build can show what Sanshain understood.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CatalogPushResult {
    pub system: String,
    pub trunk: bool,
    pub resources: Vec<String>,
    /// The `component:` refs recorded (ADR-0010), drawn as missing while no
    /// Sanshain system of that name exists.
    #[serde(default)]
    pub components: Vec<String>,
    /// Whether the push was recorded. A file Sanshain cannot read as the
    /// system's Component records nothing (the previous periphery stays) and
    /// answers `false` with the reason as a warning (ADR-0007 amendment).
    pub recorded: bool,
    /// Why nothing was recorded — travels as `Sanshain-Warning` headers.
    #[serde(skip)]
    pub warnings: Vec<String>,
    /// Whether the push changed anything (periphery or context, #48). An
    /// unchanged nightly rebuild is no event: nothing to audit.
    #[serde(skip)]
    pub changed: bool,
}

/// One entry in a provide's `harvested_subscriptions` response: what the
/// harvest did with a SUB the Producer declared (ai/improvements.md #6).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HarvestedSubscriptionResult {
    pub channel: String,
    pub message_name: String,
    /// Resolved PUB owner service name, or `None` when unfulfilled.
    pub owner: Option<String>,
    /// Drift note against the owner's GA contract when the expectation is not
    /// satisfiable. On a GA provide this is fatal (the provide is rejected, so
    /// it never reaches a success response); on a snapshot it is advisory and
    /// carried here. `None` means no contract to check or the expectation is
    /// satisfied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<String>,
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
    /// Current harvested AsyncAPI subscription edges (ai/improvements.md #6),
    /// version-less consumer edges rendered alongside the require edges.
    #[serde(default)]
    pub harvested_subscriptions: Vec<HarvestedSubscription>,
    /// Edges to Peripheral Resources (#46) for the report's scope: the dev
    /// set for the classic report, trunk or a branch's set — current or at an
    /// instant — for a scoped one. Application-populated.
    #[serde(default)]
    pub peripheral_dependencies: Vec<PeripheralDependency>,
    /// The context (#48) of every system that pushed one, by name: owner,
    /// description, source location and links. Present-day, so every scope
    /// carries the same map. Application-populated.
    #[serde(default)]
    pub systems: BTreeMap<String, SystemContext>,
    /// Trunk entries not refreshed since this instant are stale (half the
    /// trunk TTL). `None` when the TTL is disabled. Application-populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_stale_before: Option<String>,
    /// The graph scope this report describes (`main`, `<branch>[@date]`);
    /// `None` for the classic dev report. Stamped into exports so a scoped
    /// document cannot masquerade as the whole picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
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

/// The stream a build is actually recorded in, after stream resolution
/// looked at what it declared — plus the warnings for the caller when the
/// declaration could not be honoured.
#[derive(Debug, Clone, Default)]
pub struct ResolvedStream {
    /// The build is recorded in the trunk stream.
    pub trunk: bool,
    /// The build is recorded in this sanshain-branch.
    pub branch: Option<BranchInfo>,
    /// Why the declared stream was not recorded, if it was not. Answered to
    /// the caller as `Sanshain-Warning` headers.
    pub warnings: Vec<String>,
}

impl ResolvedStream {
    /// The stream label the audit trail records: `trunk`, the branch's name,
    /// or none for a dev-only build.
    pub fn label(&self) -> Option<&str> {
        if self.trunk {
            Some("trunk")
        } else {
            self.branch.as_ref().map(|b| b.name.as_str())
        }
    }
}

/// One reverse-lookup row (ADR-0005): a version of a producer that a
/// sanshain-branch currently references (pin or member version, by value).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BranchMembership {
    pub api_type: ApiType,
    pub version: SemVer,
    pub branch: String,
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
    /// The path with parameter names blanked and separators collapsed — the
    /// key the pin stores actually match on. Two spellings of one endpoint are
    /// one pin, so anything comparing pin sets must compare this, not `path`.
    /// Internal: never serialized, the wire keeps the human spelling.
    #[serde(skip)]
    pub normalized_path: String,
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
    /// What the build that provided it said about why (empty when nothing).
    #[serde(default)]
    pub origin: VersionOrigin,
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

/// Stand-in actor when a write happens with no authenticated user attached
/// (dev mode). Part of the domain because every layer that records an audit
/// row must use the same sentinel — provenance must never read as blank.
pub const DEV_MODE_ACTOR: &str = "DevMode/Anonymous";

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
    /// The declared stream (ADR-0005): `trunk`, a tag name, or `None`.
    #[serde(default)]
    pub stream: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditLogFilter {
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub action_type: Option<String>,
    pub service_wildcard: Option<String>,
    pub version_wildcard: Option<String>,
    /// Exact stream match (ADR-0005): `trunk` or a branch name. A branch name
    /// is resolved to its id before the query, so a renamed branch is found
    /// under its current name; an unresolvable name falls back to matching the
    /// recorded label, which is how a deleted branch stays reachable.
    #[serde(default)]
    pub stream: Option<String>,
    /// Set by the application layer when `stream` resolved to a live branch.
    #[serde(skip)]
    pub branch_id: Option<i64>,
    pub limit: u32,
}

// --- Webhooks (#47) ---

/// Who a hook belongs to. A global hook sees every event (narrowed by its
/// filters); a system hook sees the events of one Producer, or — on a
/// Consumer — of everything that Consumer currently requires.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WebhookScope {
    Global,
    Producer { name: String },
    Consumer { name: String },
}

impl WebhookScope {
    /// The stored `(kind, name)` pair.
    pub fn kind(&self) -> &'static str {
        match self {
            WebhookScope::Global => "global",
            WebhookScope::Producer { .. } => "producer",
            WebhookScope::Consumer { .. } => "consumer",
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            WebhookScope::Global => None,
            WebhookScope::Producer { name } | WebhookScope::Consumer { name } => Some(name),
        }
    }

    /// The stored pair back to a scope; a kind nobody wrote is `None`.
    pub fn from_stored(kind: &str, name: Option<String>) -> Option<Self> {
        match (kind, name) {
            ("global", _) => Some(WebhookScope::Global),
            ("producer", Some(name)) => Some(WebhookScope::Producer { name }),
            ("consumer", Some(name)) => Some(WebhookScope::Consumer { name }),
            _ => None,
        }
    }
}

/// The body shape a hook receives: the envelope itself, or a ready-made chat
/// message for a Slack or Teams incoming webhook. A shape, not a connector —
/// every format is one signed POST to the hook's URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WebhookFormat {
    #[default]
    Json,
    Slack,
    Teams,
}

impl WebhookFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            WebhookFormat::Json => "json",
            WebhookFormat::Slack => "slack",
            WebhookFormat::Teams => "teams",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "json" => Some(WebhookFormat::Json),
            "slack" => Some(WebhookFormat::Slack),
            "teams" => Some(WebhookFormat::Teams),
            _ => None,
        }
    }
}

/// The event catalogue: graph facts and rejections only (ADR-0009). The
/// wire names are a public contract (`site/operator/webhooks.html`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WebhookEventType {
    #[serde(rename = "version.published")]
    VersionPublished,
    #[serde(rename = "provide.rejected")]
    ProvideRejected,
    #[serde(rename = "version.deprecated")]
    VersionDeprecated,
    #[serde(rename = "dependency.recorded")]
    DependencyRecorded,
    #[serde(rename = "branch.cut")]
    BranchCut,
    #[serde(rename = "catalog.recorded")]
    CatalogRecorded,
    /// Sent by a hook's "send test event" button only, to that hook alone.
    #[serde(rename = "webhook.test")]
    Test,
}

impl WebhookEventType {
    pub const ALL: [WebhookEventType; 6] = [
        WebhookEventType::VersionPublished,
        WebhookEventType::ProvideRejected,
        WebhookEventType::VersionDeprecated,
        WebhookEventType::DependencyRecorded,
        WebhookEventType::BranchCut,
        WebhookEventType::CatalogRecorded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WebhookEventType::VersionPublished => "version.published",
            WebhookEventType::ProvideRejected => "provide.rejected",
            WebhookEventType::VersionDeprecated => "version.deprecated",
            WebhookEventType::DependencyRecorded => "dependency.recorded",
            WebhookEventType::BranchCut => "branch.cut",
            WebhookEventType::CatalogRecorded => "catalog.recorded",
            WebhookEventType::Test => "webhook.test",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "version.published" => Some(WebhookEventType::VersionPublished),
            "provide.rejected" => Some(WebhookEventType::ProvideRejected),
            "version.deprecated" => Some(WebhookEventType::VersionDeprecated),
            "dependency.recorded" => Some(WebhookEventType::DependencyRecorded),
            "branch.cut" => Some(WebhookEventType::BranchCut),
            "catalog.recorded" => Some(WebhookEventType::CatalogRecorded),
            "webhook.test" => Some(WebhookEventType::Test),
            _ => None,
        }
    }
}

/// A registered hook. The signing secret is carried for the worker and never
/// serialized; the listing shows `secret_hint`, a prefix of its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Webhook {
    pub id: i64,
    pub scope: WebhookScope,
    pub url: String,
    pub format: WebhookFormat,
    /// Empty means every event type.
    pub event_types: Vec<WebhookEventType>,
    /// `*` and `?` glob over the Producer name; `None` means every Producer.
    pub producer_glob: Option<String>,
    pub secret_hint: String,
    pub paused: bool,
    /// Registered by a caller with `manage_settings`: the egress allow-list
    /// does not apply, at registration or at delivery.
    pub allow_list_exempt: bool,
    pub created_by: String,
    pub created_at: String,
    #[serde(skip)]
    pub secret: String,
}

/// The changeable parts of a hook: pause/resume and the filters. `None`
/// leaves a field as it is; `Some(empty)` clears a filter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebhookPatch {
    pub paused: Option<bool>,
    pub event_types: Option<Vec<WebhookEventType>>,
    pub producer_glob: Option<Option<String>>,
}

/// One event for the outbox, with its fan-out already decided: one delivery
/// row per hook id. Written in the same transaction as the fact it describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWebhookEvent {
    /// UUID, the envelope's `id`.
    pub id: String,
    pub event_type: WebhookEventType,
    /// The Producer the event is about, when there is one.
    pub producer: Option<String>,
    pub occurred_at: String,
    /// The envelope's `data`, per type.
    pub data: serde_json::Value,
    pub hook_ids: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookDeliveryStatus {
    /// Never attempted, or reset by a redeliver.
    Pending,
    Delivered,
    /// The last attempt failed; another is scheduled.
    Failed,
    /// Given up: 24 hours after the event.
    Dead,
}

impl WebhookDeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WebhookDeliveryStatus::Pending => "pending",
            WebhookDeliveryStatus::Delivered => "delivered",
            WebhookDeliveryStatus::Failed => "failed",
            WebhookDeliveryStatus::Dead => "dead",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(WebhookDeliveryStatus::Pending),
            "delivered" => Some(WebhookDeliveryStatus::Delivered),
            "failed" => Some(WebhookDeliveryStatus::Failed),
            "dead" => Some(WebhookDeliveryStatus::Dead),
            _ => None,
        }
    }
}

/// One hook's delivery of one event — the delivery log row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookDelivery {
    pub id: i64,
    pub hook_id: i64,
    pub event_id: String,
    pub event_type: WebhookEventType,
    pub occurred_at: String,
    pub status: WebhookDeliveryStatus,
    pub attempts: u32,
    pub next_attempt_at: String,
    pub last_response_code: Option<u16>,
    pub last_error: Option<String>,
    pub delivered_at: Option<String>,
    /// When an operator last redelivered it by hand — grants one more
    /// attempt even past the cutoff.
    pub redelivered_at: Option<String>,
    pub updated_at: String,
}

/// A delivery the worker claimed, with everything one attempt needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueWebhookDelivery {
    pub delivery: WebhookDelivery,
    pub hook: Webhook,
    pub producer: Option<String>,
    pub data: serde_json::Value,
}

/// How one attempt ended, as the worker records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebhookAttemptOutcome {
    Delivered {
        code: u16,
    },
    /// Try again at `next_attempt_at`.
    Failed {
        code: Option<u16>,
        error: String,
        next_attempt_at: String,
    },
    /// An attempt failed after the cutoff — give up.
    Dead {
        code: Option<u16>,
        error: String,
    },
    /// Found past the cutoff before any attempt — give up without one
    /// (attempts stay as they were).
    Expired {
        error: String,
    },
}

/// What one worker pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryPassSummary {
    pub delivered: u64,
    pub failed: u64,
    pub dead: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn ldap_server_url_needs_a_scheme_a_host_and_a_valid_port() {
        let check = |url: &str| {
            serde_json::from_value::<super::LdapConfig>(serde_json::json!({
                "server_url": url, "bind_dn": "", "base_dn": ""
            }))
            .expect("minimal config")
            .validate_server_url()
        };
        for good in [
            "ldap://dc01.example.com",
            "ldaps://dc01.example.com:636",
            " ldap://10.0.0.5:389/ ",
            "ldaps://[::1]:636",
        ] {
            assert_eq!(check(good), Ok(()), "{good}");
        }
        for (bad, why) in [
            (
                "ldap:/hcwplabdc01.example.local:636",
                "no ldap:// or ldaps:// scheme",
            ),
            ("dc01.example.com:389", "no ldap:// or ldaps:// scheme"),
            ("ldap://", "names no host"),
            ("ldaps://:636", "names no host"),
            ("ldap://dc01:0", "no valid port"),
            ("ldap://dc01:70000", "no valid port"),
            ("ldap://dc01:abc", "no valid port"),
        ] {
            let err = check(bad).expect_err(bad);
            assert!(err.contains(why), "{bad}: {err}");
            assert!(err.contains("ldaps://host:port"), "{err}");
        }
    }

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

    #[test]
    fn oidc_config_validation() {
        let ok = OidcConfig {
            issuer_url: "https://idp.example.test/realms/x".into(),
            client_id: "app".into(),
            client_secret: Some("s".into()),
            redirect_url: "https://sanshain.example.test/auth/oidc/callback".into(),
            scopes: OidcConfig::default_scopes(),
            username_claim: OidcConfig::default_username_claim(),
            groups_claim: OidcConfig::default_groups_claim(),
            admin_group: String::new(),
        };
        assert!(ok.validate().is_ok());

        let mut bad_issuer = ok.clone();
        bad_issuer.issuer_url = "idp.example.test".into();
        assert_eq!(
            bad_issuer.validate().unwrap_err(),
            "Issuer URL must start with http:// or https://"
        );

        let mut no_client = ok.clone();
        no_client.client_id.clear();
        assert_eq!(
            no_client.validate().unwrap_err(),
            "Client ID must not be empty"
        );

        let mut bad_redirect = ok.clone();
        bad_redirect.redirect_url = "https://sanshain.example.test/wrong".into();
        assert_eq!(
            bad_redirect.validate().unwrap_err(),
            "Redirect URL must point at /auth/oidc/callback"
        );
    }
}
