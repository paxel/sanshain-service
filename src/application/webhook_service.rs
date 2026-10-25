//! Webhooks (#47, ADR-0009): the events Sanshain emits and who receives
//! them. This half decides *what* is written to the outbox — the event's
//! `data` per type and its fan-out — at the moment the fact it describes is
//! written; the delivery half (the worker) lives in `webhook_delivery`.

use super::now_iso;
use crate::domain::models::*;
use crate::domain::permissions::{Actor, Permission};
use crate::domain::ports::{NewWebhook, SpecRepository};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::instrument;

/// `WEBHOOKS_ENABLED=false` turns the outbox off at the source: nothing is
/// written, so re-enabling later does not replay a backlog of stale events.
/// Process-wide, like the metrics recorder, because the sources are reached
/// from every handler and background task alike.
static EMISSION_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_emission_enabled(enabled: bool) {
    EMISSION_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn emission_enabled() -> bool {
    EMISSION_ENABLED.load(Ordering::Relaxed)
}

/// The setting holding the egress allow-list: a JSON array of host patterns.
pub const ALLOWED_HOSTS_SETTING: &str = "webhook_allowed_hosts";

/// The allow-list as stored: host patterns (`hooks.slack.com`,
/// `*.webhook.office.com`), optionally prefixed `http://` to admit a plain
/// HTTP receiver inside the cluster. Empty when never set.
pub async fn webhook_allowed_hosts(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    let Some(raw) = repo.get_setting(ALLOWED_HOSTS_SETTING).await? else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&raw).map_err(|e| {
        AppError::Internal(format!(
            "the stored {ALLOWED_HOSTS_SETTING} is not a list: {e}"
        ))
    })
}

/// Replace the allow-list. A pattern is a host with `*` and `?` globs, or
/// `http://` plus such a host; anything else — a path, a port, another
/// scheme — is refused, naming the pattern.
pub async fn set_webhook_allowed_hosts(
    repo: &impl SpecRepository,
    patterns: &[String],
) -> Result<Vec<String>, AppError> {
    let mut cleaned = Vec::new();
    for pattern in patterns {
        let pattern = pattern.trim().to_ascii_lowercase();
        if pattern.is_empty() {
            continue;
        }
        let host = pattern.strip_prefix("http://").unwrap_or(&pattern);
        let valid = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '*' | '?'));
        if !valid {
            return Err(AppError::BadRequest(format!(
                "'{pattern}' is not a host pattern: use a host name with optional `*`/`?` wildcards, prefixed `http://` only to allow plain HTTP to it"
            )));
        }
        if !cleaned.contains(&pattern) {
            cleaned.push(pattern);
        }
    }
    let json = serde_json::to_string(&cleaned).map_err(|e| AppError::Internal(e.to_string()))?;
    repo.set_setting(ALLOWED_HOSTS_SETTING, &json).await?;
    Ok(cleaned)
}

/// Whether a hook URL is admitted by the allow-list: `https://` to a host
/// matching any pattern, or `http://` to a host matching a pattern that
/// says `http://` itself. Ports are not part of the match.
pub fn host_allowed(url: &str, patterns: &[String]) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    patterns.iter().any(|pattern| {
        let pattern = pattern.to_ascii_lowercase();
        match pattern.strip_prefix("http://") {
            Some(plain) => glob_matches(plain, &host),
            None => parsed.scheme() == "https" && glob_matches(&pattern, &host),
        }
    })
}

/// The registration-time check of a hook URL: well-formed and `http(s)`,
/// then — unless the caller is exempt — on the allow-list, with the
/// refusals the spec promises (an empty list says the admin has to allow
/// hosts first).
pub fn validate_hook_url(url: &str, patterns: &[String], exempt: bool) -> Result<(), AppError> {
    let parsed = url::Url::parse(url)
        .map_err(|e| AppError::BadRequest(format!("the hook URL is not a valid URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::BadRequest(
            "the hook URL must be an http:// or https:// URL with a host".to_string(),
        ));
    }
    if exempt {
        return Ok(());
    }
    if patterns.is_empty() {
        return Err(AppError::ForbiddenWithReason(
            "no hosts are allowed for webhooks yet — an administrator has to add the receiver's host to the allow-list (admin page, Webhooks) first".to_string(),
        ));
    }
    if !host_allowed(url, patterns) {
        let host = parsed.host_str().unwrap_or_default();
        let hint = if parsed.scheme() == "http" {
            " (plain http:// needs a pattern that says http:// itself)"
        } else {
            ""
        };
        return Err(AppError::ForbiddenWithReason(format!(
            "the host '{host}' is not in the webhook allow-list{hint} — ask an administrator to allow it, or use one of: {}",
            patterns.join(", ")
        )));
    }
    Ok(())
}

/// Whether `name` matches a Producer-name glob: `*` matches any run of
/// characters, `?` exactly one; everything else is literal.
pub fn glob_matches(glob: &str, name: &str) -> bool {
    // The two-pointer wildcard match: one backtrack point (the last `*`),
    // O(len(name) × len(glob)) — a recursive match is exponential on
    // `*a*a*a…`, and this runs on the request path of every provide.
    let pattern: Vec<char> = glob.chars().collect();
    let text: Vec<char> = name.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    // Both cursors only advance past an existing character, so `+ 1` stays
    // within the lengths and saturating never actually saturates.
    while let Some(&current) = text.get(t) {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p = p.saturating_add(1);
            }
            Some(&c) if c == '?' || c == current => {
                p = p.saturating_add(1);
                t = t.saturating_add(1);
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp.saturating_add(1);
                    t = st.saturating_add(1);
                    star = Some((sp, t));
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .unwrap_or_default()
        .iter()
        .all(|&c| c == '*')
}

/// Whether one hook wants this event: the scope first — a global hook sees
/// everything, a Producer hook its own Producer, a Consumer hook what it
/// currently requires — then the filters. A Producer glob admits only events
/// about a matching Producer, so a hook narrowed to a domain never sees an
/// event without one (`branch.cut`).
fn hook_wants(
    hook: &Webhook,
    event_type: WebhookEventType,
    producer: Option<&str>,
    consumers: &[String],
) -> bool {
    let in_scope = match &hook.scope {
        WebhookScope::Global => true,
        WebhookScope::Producer { name } => producer == Some(name.as_str()),
        WebhookScope::Consumer { name } => consumers.iter().any(|c| c == name),
    };
    if !in_scope {
        return false;
    }
    if !hook.event_types.is_empty() && !hook.event_types.contains(&event_type) {
        return false;
    }
    match (&hook.producer_glob, producer) {
        (None, _) => true,
        (Some(glob), Some(name)) => glob_matches(glob, name),
        (Some(_), None) => false,
    }
}

/// Who an event is about, for its fan-out: the Producer (for a catalog push,
/// the system), Consumers to include by name, and whether the Consumers
/// with a current require edge on the Producer are included too.
#[derive(Clone, Copy, Debug)]
pub struct EventAudience<'a> {
    pub producer: Option<&'a str>,
    /// Named Consumers — the one whose require is being recorded right now
    /// (its edge does not exist yet), or the system a catalog push is about.
    pub consumers: &'a [&'a str],
    /// Add every Consumer with a current require edge on `producer` (dev or
    /// trunk graph). Off for `catalog.recorded` and `branch.cut`.
    pub follow_edges: bool,
}

impl<'a> EventAudience<'a> {
    /// A Producer's event: its own hooks, its current Consumers', the global ones.
    pub fn producer(name: &'a str) -> Self {
        Self {
            producer: Some(name),
            consumers: &[],
            follow_edges: true,
        }
    }

    /// Global hooks only.
    pub fn global() -> Self {
        Self {
            producer: None,
            consumers: &[],
            follow_edges: false,
        }
    }
}

/// The hooks that want an event now — decided at the moment the fact is
/// written, never re-evaluated at delivery. Empty when emission is off.
#[instrument(skip_all, fields(event = event_type.as_str()))]
pub async fn fan_out(
    repo: &impl SpecRepository,
    event_type: WebhookEventType,
    audience: EventAudience<'_>,
) -> Result<Vec<i64>, AppError> {
    if !emission_enabled() {
        return Ok(Vec::new());
    }
    let hooks = repo.list_webhooks(None).await?;
    if hooks.is_empty() {
        return Ok(Vec::new());
    }
    let mut consumers: Vec<String> = audience.consumers.iter().map(|c| c.to_string()).collect();
    if audience.follow_edges
        && let Some(name) = audience.producer
        && let Some(service_id) = repo.find_service(name).await?
    {
        consumers.extend(repo.list_current_consumers_of(service_id).await?);
    }
    Ok(hooks
        .iter()
        .filter(|h| hook_wants(h, event_type, audience.producer, &consumers))
        .map(|h| h.id)
        .collect())
}

/// The outbox row for an event with a decided fan-out — `None` when no hook
/// wants it, so the outbox never holds a row nobody will deliver.
pub fn event_for(
    event_type: WebhookEventType,
    producer: Option<&str>,
    data: serde_json::Value,
    hook_ids: Vec<i64>,
) -> Option<NewWebhookEvent> {
    if hook_ids.is_empty() {
        return None;
    }
    Some(NewWebhookEvent {
        id: uuid::Uuid::new_v4().to_string(),
        event_type,
        producer: producer.map(str::to_string),
        occurred_at: now_iso(),
        data,
        hook_ids,
    })
}

/// Decide an event's fan-out and hand back the outbox row to write with the
/// fact — `None` when emission is off or no hook wants the event.
pub async fn plan_event(
    repo: &impl SpecRepository,
    event_type: WebhookEventType,
    audience: EventAudience<'_>,
    data: serde_json::Value,
) -> Result<Option<NewWebhookEvent>, AppError> {
    let hook_ids = fan_out(repo, event_type, audience).await?;
    Ok(event_for(event_type, audience.producer, data, hook_ids))
}

/// An event aimed at exactly one hook, filters and scope notwithstanding —
/// the "send test event" button.
pub fn test_event(hook: &Webhook) -> NewWebhookEvent {
    NewWebhookEvent {
        id: uuid::Uuid::new_v4().to_string(),
        event_type: WebhookEventType::Test,
        producer: None,
        occurred_at: now_iso(),
        data: serde_json::json!({
            "hook_id": hook.id,
            "message": "This is a test event from Sanshain. If you can read this, the receiver, the signature secret and the allow-list are all in order.",
        }),
        hook_ids: vec![hook.id],
    }
}

/// The stream label an event carries: `trunk`, a Sanshain-branch's name, or
/// `dev`.
pub fn stream_label(stream: &ResolvedStream) -> String {
    stream.label().unwrap_or("dev").to_string()
}

/// `version.published` — a Provide succeeded.
pub struct VersionPublished<'a> {
    pub producer: &'a str,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub promoted: bool,
    pub stream: &'a ResolvedStream,
    pub actor: Option<&'a str>,
    /// The classification against the previous GA, where one was computed.
    pub compatibility: Option<(SemVer, Impact)>,
    /// What the build said about why: build URL, change URL, tickets, note.
    pub origin: &'a VersionOrigin,
}

pub fn version_published_data(event: &VersionPublished<'_>) -> serde_json::Value {
    serde_json::json!({
        "producer": event.producer,
        "api_type": event.api_type.as_str(),
        "version": event.version.to_string(),
        "stability": event.stability.as_str(),
        "promoted": event.promoted,
        "stream": stream_label(event.stream),
        "actor": event.actor,
        "compatibility": event.compatibility.map(|(against, impact)| serde_json::json!({
            "against": against.to_string(),
            "impact": impact_label(impact),
        })),
        "origin": event.origin,
    })
}

fn impact_label(impact: Impact) -> &'static str {
    match impact {
        Impact::None => "none",
        Impact::Patch => "patch",
        Impact::Minor => "minor",
        Impact::Major => "major",
    }
}

/// `provide.rejected` — a Provide refused by a version rule.
pub struct ProvideRejected<'a> {
    pub producer: &'a str,
    pub api_type: ApiType,
    pub version: SemVer,
    pub rule: &'a str,
    pub message: &'a str,
    pub proposed_version: Option<SemVer>,
    pub actor: Option<&'a str>,
}

pub fn provide_rejected_data(event: &ProvideRejected<'_>) -> serde_json::Value {
    serde_json::json!({
        "producer": event.producer,
        "api_type": event.api_type.as_str(),
        "version": event.version.to_string(),
        "rule": event.rule,
        "message": event.message,
        "proposed_version": event.proposed_version.map(|v| v.to_string()),
        "actor": event.actor,
    })
}

/// The elements a Provide newly marks deprecated: deprecated now and not in
/// the previous GA (every deprecated element when there is no previous GA).
/// Empty when nothing is new. The second answer says whether the whole
/// document is deprecated — every element it has.
pub fn newly_deprecated(
    endpoints: &[EndpointRecord],
    previous_ga: Option<&[EndpointRecord]>,
) -> (Vec<(String, String)>, bool) {
    let was_deprecated = |e: &EndpointRecord| {
        previous_ga.is_some_and(|old| {
            old.iter().any(|o| {
                o.api_type == e.api_type
                    && o.normalized_path == e.normalized_path
                    && o.method == e.method
                    && o.deprecated
            })
        })
    };
    let new: Vec<(String, String)> = endpoints
        .iter()
        .filter(|e| e.deprecated && !was_deprecated(e))
        .map(|e| (e.path.clone(), e.method.clone()))
        .collect();
    let whole = !endpoints.is_empty() && endpoints.iter().all(|e| e.deprecated);
    (new, whole)
}

pub fn version_deprecated_data(
    producer: &str,
    api_type: ApiType,
    version: SemVer,
    stability: Stability,
    elements: &[(String, String)],
    document_deprecated: bool,
) -> serde_json::Value {
    serde_json::json!({
        "producer": producer,
        "api_type": api_type.as_str(),
        "version": version.to_string(),
        "stability": stability.as_str(),
        "elements": elements.iter().map(|(path, method)| serde_json::json!({
            "path": path,
            "method": method,
        })).collect::<Vec<_>>(),
        "document_deprecated": document_deprecated,
    })
}

/// `dependency.recorded` — a require recorded a new edge.
pub struct DependencyRecorded<'a> {
    pub consumer: &'a str,
    pub producer: &'a str,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub endpoints: &'a [(String, String)],
    pub stream: &'a ResolvedStream,
}

pub fn dependency_recorded_data(event: &DependencyRecorded<'_>) -> serde_json::Value {
    serde_json::json!({
        "consumer": event.consumer,
        "producer": event.producer,
        "api_type": event.api_type.as_str(),
        "version": event.version.to_string(),
        "stability": event.stability.as_str(),
        "snapshot_pinned": event.stability == Stability::Snapshot,
        "stream": stream_label(event.stream),
        "endpoints": event.endpoints.iter().map(|(path, method)| serde_json::json!({
            "path": path,
            "method": method,
        })).collect::<Vec<_>>(),
    })
}

/// `branch.cut` — a Sanshain-branch was created.
pub fn branch_cut_data(
    name: &str,
    source: &str,
    as_of: &str,
    cut_at: &str,
    created_by: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "source": source,
        "as_of": as_of,
        "cut_at": cut_at,
        "actor": created_by,
    })
}

/// `catalog.recorded` — a catalog push ran.
pub fn catalog_recorded_data(
    system: &str,
    trunk: bool,
    recorded: bool,
    warning: Option<&str>,
    resources: usize,
) -> serde_json::Value {
    serde_json::json!({
        "system": system,
        "trunk": trunk,
        "recorded": recorded,
        "warning": warning,
        "resources": resources,
    })
}

// --- Hook management ---

/// What a caller sends to register a hook.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWebhookRequest {
    pub url: String,
    #[serde(default)]
    pub format: WebhookFormat,
    /// Empty or absent: every event type.
    #[serde(default)]
    pub event_types: Vec<WebhookEventType>,
    #[serde(default)]
    pub producer_glob: Option<String>,
}

/// A registered hook with its secret — the one time the secret is shown.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CreatedWebhook {
    pub webhook: Webhook,
    pub secret: String,
}

/// 32 random bytes as `whsec_<hex>`, and the hint the listing shows: the
/// first 8 hex characters of the secret's SHA-256.
fn new_secret() -> (String, String) {
    let mut bytes = [0u8; 32];
    {
        use rand::RngExt;
        rand::rng().fill(&mut bytes);
    }
    let secret = format!("whsec_{}", hex::encode(bytes));
    (secret_hint(&secret), secret)
}

fn secret_hint(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let hash = hex::encode(hasher.finalize());
    hash.get(..8).unwrap_or(&hash).to_string()
}

fn validate_glob(glob: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(glob) = glob.map(str::trim).filter(|g| !g.is_empty()) else {
        return Ok(None);
    };
    if glob.len() > 200 {
        return Err(AppError::BadRequest(
            "the Producer glob is too long (200 characters at most)".to_string(),
        ));
    }
    Ok(Some(glob.to_string()))
}

/// The scope must name a system that exists: the Producer, or — for a
/// Consumer hook — the Consumer or a Producer of the same name (the system
/// may not have required anything yet).
async fn check_scope_exists(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
) -> Result<(), AppError> {
    match scope {
        WebhookScope::Global => Ok(()),
        WebhookScope::Producer { name } => {
            if repo.find_service(name).await?.is_none() {
                return Err(AppError::NotFound(format!("Producer '{name}' is unknown")));
            }
            Ok(())
        }
        WebhookScope::Consumer { name } => {
            let known = repo.list_consumers().await?.iter().any(|c| c == name)
                || repo.find_service(name).await?.is_some();
            if !known {
                return Err(AppError::NotFound(format!("Consumer '{name}' is unknown")));
            }
            Ok(())
        }
    }
}

/// Register a hook in `scope`. The URL is checked against the allow-list
/// unless the Actor holds `manage_settings`, in which case the hook is
/// exempt for good — at registration and at every delivery.
#[instrument(skip_all, fields(scope = scope.kind()))]
pub async fn create_webhook(
    repo: &impl SpecRepository,
    actor: &Actor,
    scope: WebhookScope,
    request: CreateWebhookRequest,
) -> Result<CreatedWebhook, AppError> {
    check_scope_exists(repo, &scope).await?;
    let url = request.url.trim().to_string();
    let exempt = actor.has_permission(Permission::ManageSettings);
    let allowed = webhook_allowed_hosts(repo).await?;
    validate_hook_url(&url, &allowed, exempt)?;
    let producer_glob = validate_glob(request.producer_glob.as_deref())?;
    let mut event_types = request.event_types;
    event_types.sort_by_key(|t| t.as_str());
    event_types.dedup();
    if event_types.contains(&WebhookEventType::Test) {
        return Err(AppError::BadRequest(
            "webhook.test is sent by the test button only and cannot be subscribed to".to_string(),
        ));
    }
    let (hint, secret) = new_secret();
    let created_at = now_iso();
    let id = repo
        .create_webhook(NewWebhook {
            scope: &scope,
            url: &url,
            format: request.format,
            event_types: &event_types,
            producer_glob: producer_glob.as_deref(),
            secret: &secret,
            secret_hint: &hint,
            allow_list_exempt: exempt,
            created_by: &actor.username,
            created_at: &created_at,
        })
        .await?;
    let webhook = repo
        .get_webhook(id)
        .await?
        .ok_or_else(|| AppError::Internal("the hook vanished after creation".to_string()))?;
    Ok(CreatedWebhook { webhook, secret })
}

pub async fn list_webhooks_in(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
) -> Result<Vec<Webhook>, AppError> {
    Ok(repo.list_webhooks(Some(scope)).await?)
}

/// A hook by id, in this scope only: a system route never reaches a global
/// hook or another system's, whatever the id says.
pub async fn get_webhook_in(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
) -> Result<Webhook, AppError> {
    match repo.get_webhook(id).await? {
        Some(hook) if &hook.scope == scope => Ok(hook),
        _ => Err(AppError::NotFound(format!("no webhook {id} in this scope"))),
    }
}

/// The patch a caller sends: pause/resume, and the filters. `event_types:
/// []` means every type; `producer_glob: null` clears the glob.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchWebhookRequest {
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub event_types: Option<Vec<WebhookEventType>>,
    #[serde(default, deserialize_with = "double_option")]
    pub producer_glob: Option<Option<String>>,
}

fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <Option<String> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

#[instrument(skip_all, fields(scope = scope.kind(), id))]
pub async fn patch_webhook(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
    request: PatchWebhookRequest,
) -> Result<Webhook, AppError> {
    get_webhook_in(repo, scope, id).await?;
    let event_types = match request.event_types {
        Some(mut types) => {
            if types.contains(&WebhookEventType::Test) {
                return Err(AppError::BadRequest(
                    "webhook.test is sent by the test button only and cannot be subscribed to"
                        .to_string(),
                ));
            }
            types.sort_by_key(|t| t.as_str());
            types.dedup();
            Some(types)
        }
        None => None,
    };
    let producer_glob = match request.producer_glob {
        Some(glob) => Some(validate_glob(glob.as_deref())?),
        None => None,
    };
    repo.update_webhook(
        id,
        &WebhookPatch {
            paused: request.paused,
            event_types,
            producer_glob,
        },
    )
    .await?;
    get_webhook_in(repo, scope, id).await
}

/// A fresh secret, shown once like the first; deliveries from now on are
/// signed with it.
#[instrument(skip_all, fields(scope = scope.kind(), id))]
pub async fn rotate_webhook_secret(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
) -> Result<CreatedWebhook, AppError> {
    get_webhook_in(repo, scope, id).await?;
    let (hint, secret) = new_secret();
    repo.rotate_webhook_secret(id, &secret, &hint).await?;
    let webhook = get_webhook_in(repo, scope, id).await?;
    Ok(CreatedWebhook { webhook, secret })
}

#[instrument(skip_all, fields(scope = scope.kind(), id))]
pub async fn delete_webhook_in(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
) -> Result<Webhook, AppError> {
    let hook = get_webhook_in(repo, scope, id).await?;
    repo.delete_webhook(id).await?;
    Ok(hook)
}

/// Queue a `webhook.test` event for this hook alone; the worker delivers it
/// like any other, so the signature, the format and the allow-list are all
/// exercised. Answers the event id the delivery log will show.
#[instrument(skip_all, fields(scope = scope.kind(), id))]
pub async fn send_test_event(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
) -> Result<String, AppError> {
    let hook = get_webhook_in(repo, scope, id).await?;
    if !emission_enabled() {
        return Err(AppError::Conflict(
            "webhooks are disabled on this instance (WEBHOOKS_ENABLED=false): a test event would never be delivered".to_string(),
        ));
    }
    let event = test_event(&hook);
    let event_id = event.id.clone();
    repo.insert_webhook_events(&[event]).await?;
    Ok(event_id)
}

/// The newest deliveries of a hook, at most 200.
pub async fn list_deliveries_in(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
    limit: Option<u32>,
) -> Result<Vec<WebhookDelivery>, AppError> {
    get_webhook_in(repo, scope, id).await?;
    Ok(repo
        .list_webhook_deliveries(id, limit.unwrap_or(50).clamp(1, 200))
        .await?)
}

/// Redeliver one delivery of this hook: pending again, due now, attempt
/// count kept — one more attempt even past the 24 h cutoff. A delivery a
/// worker currently holds is refused with a `409`: resetting it would send
/// it twice.
#[instrument(skip_all, fields(scope = scope.kind(), id, delivery_id))]
pub async fn redeliver(
    repo: &impl SpecRepository,
    scope: &WebhookScope,
    id: i64,
    delivery_id: i64,
) -> Result<WebhookDelivery, AppError> {
    get_webhook_in(repo, scope, id).await?;
    match repo.get_webhook_delivery(delivery_id).await? {
        Some(delivery) if delivery.hook_id == id => {}
        _ => {
            return Err(AppError::NotFound(format!(
                "no delivery {delivery_id} on webhook {id}"
            )));
        }
    }
    if !repo.reset_webhook_delivery(delivery_id, &now_iso()).await? {
        return Err(AppError::Conflict(format!(
            "delivery {delivery_id} is in flight — a worker is attempting it right now; try again after that attempt"
        )));
    }
    repo.get_webhook_delivery(delivery_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no delivery {delivery_id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;
    use crate::domain::ports::NewWebhook;

    fn hook(id: i64, scope: WebhookScope) -> Webhook {
        Webhook {
            id,
            scope,
            url: "https://hooks.example/x".to_string(),
            format: WebhookFormat::Json,
            event_types: Vec::new(),
            producer_glob: None,
            secret_hint: "hint".to_string(),
            paused: false,
            allow_list_exempt: true,
            created_by: "root".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            secret: "whsec_x".to_string(),
        }
    }

    #[test]
    fn globs_support_star_and_question_mark_only() {
        assert!(glob_matches("payments-*", "payments-core"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("order?", "orders"));
        assert!(!glob_matches("order?", "order"));
        assert!(!glob_matches("payments-*", "billing"));
        assert!(glob_matches("a*b*c", "aXXbYYc"));
        assert!(!glob_matches("[a-z]+", "abc"), "no character classes");
        assert!(glob_matches("exact", "exact"));
    }

    #[test]
    fn scope_and_filters_decide_who_wants_an_event() {
        let consumers = vec!["web".to_string()];
        let global = hook(1, WebhookScope::Global);
        let orders = hook(
            2,
            WebhookScope::Producer {
                name: "orders".to_string(),
            },
        );
        let web = hook(
            3,
            WebhookScope::Consumer {
                name: "web".to_string(),
            },
        );
        let cli = hook(
            4,
            WebhookScope::Consumer {
                name: "cli".to_string(),
            },
        );
        let published = WebhookEventType::VersionPublished;
        assert!(hook_wants(&global, published, Some("orders"), &consumers));
        assert!(hook_wants(&orders, published, Some("orders"), &consumers));
        assert!(!hook_wants(&orders, published, Some("billing"), &consumers));
        assert!(hook_wants(&web, published, Some("orders"), &consumers));
        assert!(!hook_wants(&cli, published, Some("orders"), &consumers));

        // branch.cut has no Producer: global hooks only.
        let cut = WebhookEventType::BranchCut;
        assert!(hook_wants(&global, cut, None, &[]));
        assert!(!hook_wants(&orders, cut, None, &[]));

        let mut filtered = hook(5, WebhookScope::Global);
        filtered.event_types = vec![WebhookEventType::ProvideRejected];
        assert!(!hook_wants(
            &filtered,
            published,
            Some("orders"),
            &consumers
        ));
        assert!(hook_wants(
            &filtered,
            WebhookEventType::ProvideRejected,
            Some("orders"),
            &consumers
        ));

        let mut domain = hook(6, WebhookScope::Global);
        domain.producer_glob = Some("pay*".to_string());
        assert!(hook_wants(&domain, published, Some("payments"), &[]));
        assert!(!hook_wants(&domain, published, Some("orders"), &[]));
        assert!(
            !hook_wants(&domain, cut, None, &[]),
            "a domain hook never sees an event without a Producer"
        );
    }

    #[tokio::test]
    async fn planning_resolves_consumers_and_skips_when_disabled_or_hookless() {
        let repo = MockRepo::new();
        let none = plan_event(
            &repo,
            WebhookEventType::BranchCut,
            EventAudience::global(),
            serde_json::json!({}),
        )
        .await
        .unwrap();
        assert!(none.is_none(), "no hooks, no outbox row");

        let scope = WebhookScope::Consumer {
            name: "web".to_string(),
        };
        let id = repo
            .create_webhook(NewWebhook {
                scope: &scope,
                url: "https://hooks.example/x",
                format: WebhookFormat::Json,
                event_types: &[],
                producer_glob: None,
                secret: "s",
                secret_hint: "h",
                allow_list_exempt: true,
                created_by: "root",
                created_at: "2026-01-01T00:00:00Z",
            })
            .await
            .unwrap();
        // A hook exists but wants none of this: no orphan outbox row.
        let unwanted = plan_event(
            &repo,
            WebhookEventType::BranchCut,
            EventAudience::global(),
            serde_json::json!({}),
        )
        .await
        .unwrap();
        assert!(unwanted.is_none(), "no hook wants it, no outbox row");

        let planned = plan_event(
            &repo,
            WebhookEventType::DependencyRecorded,
            EventAudience {
                producer: Some("orders"),
                consumers: &["web"],
                follow_edges: true,
            },
            serde_json::json!({"consumer": "web"}),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(planned.hook_ids, vec![id]);
        assert_eq!(planned.producer.as_deref(), Some("orders"));
        assert_eq!(planned.event_type, WebhookEventType::DependencyRecorded);
        assert!(!planned.id.is_empty());

        set_emission_enabled(false);
        let off = plan_event(
            &repo,
            WebhookEventType::DependencyRecorded,
            EventAudience {
                producer: Some("orders"),
                consumers: &["web"],
                follow_edges: true,
            },
            serde_json::json!({}),
        )
        .await
        .unwrap();
        let test_refused = send_test_event(&repo, &scope, id).await;
        set_emission_enabled(true);
        assert!(off.is_none());
        assert!(
            matches!(test_refused, Err(AppError::Conflict(ref m)) if m.contains("WEBHOOKS_ENABLED=false")),
            "a test event while disabled is refused, not queued forever: {test_refused:?}"
        );
    }

    #[tokio::test]
    async fn follow_edges_off_keeps_a_producers_consumers_out() {
        let repo = MockRepo::new();
        repo.ensure_service("orders").await.unwrap();
        let web = WebhookScope::Consumer {
            name: "web".to_string(),
        };
        let own = WebhookScope::Producer {
            name: "orders".to_string(),
        };
        let mut ids = Vec::new();
        for scope in [&web, &own] {
            ids.push(
                repo.create_webhook(NewWebhook {
                    scope,
                    url: "https://hooks.example/x",
                    format: WebhookFormat::Json,
                    event_types: &[],
                    producer_glob: None,
                    secret: "s",
                    secret_hint: "h",
                    allow_list_exempt: true,
                    created_by: "root",
                    created_at: "2026-01-01T00:00:00Z",
                })
                .await
                .unwrap(),
            );
        }
        // web is a current Consumer of orders (named, as a require would).
        let with_edges = fan_out(
            &repo,
            WebhookEventType::VersionPublished,
            EventAudience {
                producer: Some("orders"),
                consumers: &["web"],
                follow_edges: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(with_edges, ids);
        let catalog = fan_out(
            &repo,
            WebhookEventType::CatalogRecorded,
            EventAudience {
                producer: Some("orders"),
                consumers: &["orders"],
                follow_edges: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(catalog, vec![ids[1]], "the system's own hooks only");
    }

    #[test]
    fn a_pathological_glob_matches_in_linear_time() {
        let name = "a".repeat(200);
        let glob = "*a".repeat(30) + "b";
        let started = std::time::Instant::now();
        assert!(!glob_matches(&glob, &name));
        assert!(glob_matches(&"*a".repeat(30), &name));
        assert!(glob_matches("*?*?*?*?*?*", &name));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_allow_list_admits_https_to_listed_hosts_and_http_only_when_said_so() {
        let patterns = vec![
            "hooks.slack.com".to_string(),
            "*.webhook.office.com".to_string(),
            "http://n8n.corp.internal".to_string(),
        ];
        assert!(host_allowed(
            "https://hooks.slack.com/services/T/B/x",
            &patterns
        ));
        assert!(host_allowed("https://HOOKS.slack.com/x", &patterns));
        assert!(host_allowed(
            "https://corp.webhook.office.com/webhookb2/x",
            &patterns
        ));
        assert!(
            !host_allowed("https://webhook.office.com/x", &patterns),
            "the glob needs a label"
        );
        assert!(
            !host_allowed("http://hooks.slack.com/x", &patterns),
            "plain http needs an http pattern"
        );
        assert!(
            host_allowed("http://n8n.corp.internal:5678/hook", &patterns),
            "ports are not matched"
        );
        assert!(
            host_allowed("https://n8n.corp.internal/hook", &patterns),
            "an http pattern admits https too"
        );
        assert!(!host_allowed("https://evil.example/x", &patterns));
        assert!(!host_allowed(
            "https://kubernetes.default.svc/api",
            &patterns
        ));
        assert!(!host_allowed("not a url", &patterns));

        assert!(
            validate_hook_url("https://evil.example/x", &patterns, true).is_ok(),
            "exempt"
        );
        match validate_hook_url("https://evil.example/x", &patterns, false) {
            Err(AppError::ForbiddenWithReason(msg)) => assert!(
                msg.contains("'evil.example' is not in the webhook allow-list"),
                "{msg}"
            ),
            other => panic!("expected a refusal, got {other:?}"),
        }
        match validate_hook_url("https://hooks.slack.com/x", &[], false) {
            Err(AppError::ForbiddenWithReason(msg)) => {
                assert!(msg.contains("no hosts are allowed"), "{msg}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(matches!(
            validate_hook_url("ftp://x.example/y", &patterns, true),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_hook_url("nope", &patterns, true),
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn the_allow_list_setting_round_trips_and_refuses_junk() {
        let repo = MockRepo::new();
        assert!(webhook_allowed_hosts(&repo).await.unwrap().is_empty());
        let stored = set_webhook_allowed_hosts(
            &repo,
            &[
                " Hooks.Slack.com ".to_string(),
                "".to_string(),
                "http://n8n.corp.internal".to_string(),
                "hooks.slack.com".to_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            stored,
            vec![
                "hooks.slack.com".to_string(),
                "http://n8n.corp.internal".to_string()
            ]
        );
        assert_eq!(webhook_allowed_hosts(&repo).await.unwrap(), stored);
        for junk in [
            "https://hooks.slack.com",
            "hooks.slack.com/path",
            "hooks.slack.com:443",
            "ho st",
        ] {
            assert!(
                matches!(
                    set_webhook_allowed_hosts(&repo, &[junk.to_string()]).await,
                    Err(AppError::BadRequest(_))
                ),
                "{junk}"
            );
        }
    }

    #[test]
    fn a_new_secret_is_32_random_bytes_with_a_hash_prefix_hint() {
        let (hint, secret) = new_secret();
        assert!(secret.starts_with("whsec_"));
        assert_eq!(secret.len(), "whsec_".len() + 64);
        assert_eq!(hint.len(), 8);
        assert_eq!(hint, secret_hint(&secret));
        assert_ne!(new_secret().1, secret);
    }

    #[tokio::test]
    async fn hook_management_round_trip_keeps_scopes_apart() {
        let repo = MockRepo::new();
        repo.ensure_service("orders").await.unwrap();
        let admin = Actor {
            user_id: 0,
            username: "root".to_string(),
            is_root: true,
            roles: Vec::new(),
            directory_groups: Vec::new(),
        };
        let request = || CreateWebhookRequest {
            url: "https://hooks.example/x".to_string(),
            format: WebhookFormat::Slack,
            event_types: vec![
                WebhookEventType::VersionPublished,
                WebhookEventType::VersionPublished,
            ],
            producer_glob: Some(" pay* ".to_string()),
        };
        // Root holds manage_settings: exempt from the (empty) allow-list.
        let created = create_webhook(&repo, &admin, WebhookScope::Global, request())
            .await
            .unwrap();
        assert!(created.webhook.allow_list_exempt);
        assert_eq!(
            created.webhook.event_types,
            vec![WebhookEventType::VersionPublished]
        );
        assert_eq!(created.webhook.producer_glob.as_deref(), Some("pay*"));
        assert_eq!(created.webhook.format, WebhookFormat::Slack);
        let id = created.webhook.id;

        let producer_scope = WebhookScope::Producer {
            name: "orders".to_string(),
        };
        assert!(matches!(
            get_webhook_in(&repo, &producer_scope, id).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            patch_webhook(&repo, &producer_scope, id, PatchWebhookRequest::default()).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            delete_webhook_in(&repo, &producer_scope, id).await,
            Err(AppError::NotFound(_))
        ));
        let unknown = WebhookScope::Producer {
            name: "nope".to_string(),
        };
        assert!(matches!(
            create_webhook(&repo, &admin, unknown, request()).await,
            Err(AppError::NotFound(_))
        ));

        let patched = patch_webhook(
            &repo,
            &WebhookScope::Global,
            id,
            PatchWebhookRequest {
                paused: Some(true),
                event_types: Some(Vec::new()),
                producer_glob: Some(None),
            },
        )
        .await
        .unwrap();
        assert!(patched.paused);
        assert!(patched.event_types.is_empty());
        assert_eq!(patched.producer_glob, None);

        let rotated = rotate_webhook_secret(&repo, &WebhookScope::Global, id)
            .await
            .unwrap();
        assert_ne!(rotated.secret, created.secret);
        assert_ne!(rotated.webhook.secret_hint, created.webhook.secret_hint);

        let event_id = send_test_event(&repo, &WebhookScope::Global, id)
            .await
            .unwrap();
        let rows = list_deliveries_in(&repo, &WebhookScope::Global, id, None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_id, event_id);
        assert_eq!(rows[0].event_type, WebhookEventType::Test);
        let again = redeliver(&repo, &WebhookScope::Global, id, rows[0].id)
            .await
            .unwrap();
        assert_eq!(again.status, WebhookDeliveryStatus::Pending);
        assert!(again.redelivered_at.is_some());
        // A worker holds it now (the hook resumed first): redelivering would
        // send it twice.
        patch_webhook(
            &repo,
            &WebhookScope::Global,
            id,
            PatchWebhookRequest {
                paused: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let claimed = repo
            .claim_due_webhook_deliveries("2999-01-01T00:00:00Z", "2999-01-01T00:05:00Z", 10)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(matches!(
            redeliver(&repo, &WebhookScope::Global, id, rows[0].id).await,
            Err(AppError::Conflict(ref m)) if m.contains("in flight")
        ));
        assert!(matches!(
            redeliver(&repo, &WebhookScope::Global, id, 999_999).await,
            Err(AppError::NotFound(_))
        ));

        assert_eq!(
            delete_webhook_in(&repo, &WebhookScope::Global, id)
                .await
                .unwrap()
                .id,
            id
        );
        assert!(
            list_webhooks_in(&repo, &WebhookScope::Global)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn newly_deprecated_compares_against_the_previous_ga() {
        let ep = |path: &str, deprecated: bool| EndpointRecord {
            id: None,
            api_type: ApiType::OpenApi,
            path: path.to_string(),
            normalized_path: path.to_string(),
            method: "GET".to_string(),
            yaml_content: String::new(),
            deprecated,
        };
        let old = vec![ep("/a", true), ep("/b", false)];
        let new = vec![ep("/a", true), ep("/b", true), ep("/c", false)];
        let (elements, whole) = newly_deprecated(&new, Some(&old));
        assert_eq!(elements, vec![("/b".to_string(), "GET".to_string())]);
        assert!(!whole);
        let (elements, whole) = newly_deprecated(&[ep("/a", true)], None);
        assert_eq!(elements.len(), 1);
        assert!(whole, "every element deprecated = the document is");
        let (elements, _) = newly_deprecated(&[ep("/a", false)], None);
        assert!(elements.is_empty());
    }

    #[test]
    fn event_data_carries_the_documented_fields() {
        let stream = ResolvedStream {
            trunk: true,
            branch: None,
            warnings: Vec::new(),
        };
        let data = version_published_data(&VersionPublished {
            producer: "orders",
            api_type: ApiType::OpenApi,
            version: SemVer::new(2, 0, 0),
            stability: Stability::Ga,
            promoted: true,
            stream: &stream,
            actor: Some("alice"),
            compatibility: Some((SemVer::new(1, 4, 0), Impact::Major)),
            origin: &VersionOrigin {
                build_url: Some("https://ci.acme.corp/job/orders/42".to_string()),
                change_url: None,
                tickets: vec!["ORD-7".to_string()],
                note: Some("Ship v2".to_string()),
                trunk_build: false,
            },
        });
        assert_eq!(
            data,
            serde_json::json!({
                "producer": "orders", "api_type": "openapi", "version": "2.0.0",
                "stability": "ga", "promoted": true, "stream": "trunk", "actor": "alice",
                "compatibility": {"against": "1.4.0", "impact": "major"},
                "origin": {
                    "build_url": "https://ci.acme.corp/job/orders/42", "change_url": null,
                    "tickets": ["ORD-7"], "note": "Ship v2", "trunk_build": false,
                },
            })
        );
        let data = dependency_recorded_data(&DependencyRecorded {
            consumer: "web",
            producer: "orders",
            api_type: ApiType::OpenApi,
            version: SemVer::new(1, 0, 0),
            stability: Stability::Snapshot,
            endpoints: &[("/a".to_string(), "GET".to_string())],
            stream: &ResolvedStream::default(),
        });
        assert_eq!(data["snapshot_pinned"], true);
        assert_eq!(data["stream"], "dev");
        assert_eq!(data["endpoints"][0]["path"], "/a");
        let data = catalog_recorded_data("orders", false, false, Some("not valid YAML"), 0);
        assert_eq!(data["recorded"], false);
        assert_eq!(data["warning"], "not valid YAML");
        let data = branch_cut_data(
            "rel",
            "trunk",
            "2026-01-01T00:00:00Z",
            "2026-01-02T00:00:00Z",
            "bob",
        );
        assert_eq!(data["name"], "rel");
        assert_eq!(data["actor"], "bob");
        let data = provide_rejected_data(&ProvideRejected {
            producer: "orders",
            api_type: ApiType::Proto,
            version: SemVer::new(1, 0, 0),
            rule: "immutable_ga",
            message: "nope",
            proposed_version: Some(SemVer::new(1, 0, 1)),
            actor: None,
        });
        assert_eq!(data["proposed_version"], "1.0.1");
        assert_eq!(data["actor"], serde_json::Value::Null);
    }
}
