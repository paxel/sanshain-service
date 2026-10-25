//! Webhooks (#47, ADR-0009), the delivery half: one worker pass claims the
//! due deliveries, renders each hook's body, signs it, POSTs it through the
//! [`WebhookSender`] port and records the outcome with the next attempt. The
//! pass is a plain function with an injectable clock, so backoff and the
//! cutoff are tested without waiting; `main` runs it on an interval.

use super::webhook_service::{host_allowed, webhook_allowed_hosts};
use crate::domain::models::*;
use crate::domain::ports::{SpecRepository, WebhookRequest, WebhookSender};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tracing::instrument;

/// First retry after 30 s, doubling to at most an hour between attempts.
const FIRST_BACKOFF_SECS: i64 = 30;
const MAX_BACKOFF_SECS: i64 = 3600;
/// A delivery is given up 24 h after its event occurred — measured from the
/// event, not from the first attempt, so a hook paused for a day finds its
/// deliveries dead on resume.
const CUTOFF_HOURS: i64 = 24;
/// Added to the longest a pass can take (batch × delivery timeout) for the
/// lease a claim holds: long enough that a slow pass never loses its rows
/// to another replica, short enough that a crashed pass loses nothing for
/// good.
const LEASE_MARGIN_SECS: i64 = 60;
/// Delivered and dead rows are kept this long for the delivery log.
pub const RETENTION_DAYS: i64 = 30;

/// What the worker needs beyond the repository: the public base URL for
/// deep links (`PUBLIC_BASE_URL`, or the OIDC redirect URL's origin, or none),
/// how many deliveries one pass claims, and the sender's timeout — the two
/// bound how long a pass can take, which sizes the claim's lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookDeliveryConfig {
    pub public_base_url: Option<String>,
    pub batch_size: u32,
    pub delivery_timeout_secs: u64,
}

impl Default for WebhookDeliveryConfig {
    fn default() -> Self {
        Self {
            public_base_url: None,
            batch_size: 50,
            delivery_timeout_secs: 10,
        }
    }
}

impl WebhookDeliveryConfig {
    /// How long a claim holds its rows: the worst-case pass plus a margin.
    pub fn lease_secs(&self) -> i64 {
        i64::from(self.batch_size)
            .saturating_mul(self.delivery_timeout_secs.min(i64::MAX as u64) as i64)
            .saturating_add(LEASE_MARGIN_SECS)
    }
}

/// Slack reads `&`, `<` and `>` as markup in message text; a Producer named
/// `<https://evil|orders>` would render as a link. Escaped in the summary,
/// never in the link Sanshain builds itself.
pub fn slack_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Adaptive Card text blocks read a markdown subset (`**bold**`, `_italic_`,
/// `[text](url)`); a backslash escapes those characters, so caller-supplied
/// names cannot smuggle a link into the card.
pub fn teams_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Seconds to wait before the next attempt, given how many were made.
pub fn backoff_secs(attempts_made: u32) -> i64 {
    let doubled = FIRST_BACKOFF_SECS.saturating_mul(1i64 << attempts_made.min(30));
    doubled.min(MAX_BACKOFF_SECS)
}

/// The hex HMAC-SHA256 of `"<timestamp>.<body>"` under the hook's secret.
pub fn signature(secret: &str, timestamp: i64, body: &str) -> String {
    // HMAC accepts a key of any length, so this never fails; kept total
    // rather than `expect`ed per the no-panic policy.
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The `Sanshain-Signature` header value: `t=<unix seconds>,v1=<hex>`.
pub fn signature_header(secret: &str, timestamp: i64, body: &str) -> String {
    format!("t={timestamp},v1={}", signature(secret, timestamp, body))
}

/// The origin (`scheme://host[:port]`) of a URL, for the deep-link fallback.
pub fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let mut origin = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        origin.push_str(&format!(":{port}"));
    }
    Some(origin)
}

/// The base URL deep links are built on: the configured one, else the
/// origin of the stored OIDC redirect URL, else none (links are omitted).
pub async fn resolve_base_url(
    repo: &impl SpecRepository,
    configured: Option<&str>,
) -> Result<Option<String>, AppError> {
    if let Some(url) = configured {
        return Ok(Some(url.trim_end_matches('/').to_string()));
    }
    Ok(super::auth_service::get_oidc_config(repo)
        .await?
        .and_then(|c| origin_of(&c.redirect_url)))
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The deep links an event carries, per type, on `base`.
pub fn links_for(
    event_type: WebhookEventType,
    data: &serde_json::Value,
    base: &str,
) -> serde_json::Value {
    let field = |name: &str| data.get(name).and_then(|v| v.as_str()).unwrap_or("");
    let producer_page = |name: &str| format!("{base}/producers.html?service={}", enc(name));
    let version_page = || {
        format!(
            "{base}/producers.html?service={}&api_type={}&version={}",
            enc(field("producer")),
            enc(field("api_type")),
            enc(field("version"))
        )
    };
    let graph = format!("{base}/graph.html");
    match event_type {
        WebhookEventType::VersionPublished | WebhookEventType::VersionDeprecated => {
            serde_json::json!({
                "producer": producer_page(field("producer")),
                "version": version_page(),
                "graph": graph,
            })
        }
        WebhookEventType::ProvideRejected => serde_json::json!({
            "producer": producer_page(field("producer")),
            "audit": format!("{base}/audit.html"),
        }),
        WebhookEventType::DependencyRecorded => serde_json::json!({
            "consumer": format!("{base}/consumers.html?name={}", enc(field("consumer"))),
            "producer": producer_page(field("producer")),
            "version": version_page(),
            "graph": graph,
        }),
        WebhookEventType::BranchCut => serde_json::json!({ "graph": graph }),
        WebhookEventType::CatalogRecorded => serde_json::json!({
            "producer": producer_page(field("system")),
            "consumer": format!("{base}/consumers.html?name={}", enc(field("system"))),
            "graph": graph,
        }),
        WebhookEventType::Test => serde_json::json!({ "admin": format!("{base}/admin.html") }),
    }
}

/// The one link a chat message opens: the most specific of the event's.
fn primary_link(event_type: WebhookEventType, links: Option<&serde_json::Value>) -> Option<String> {
    let links = links?;
    let key = match event_type {
        WebhookEventType::VersionPublished
        | WebhookEventType::VersionDeprecated
        | WebhookEventType::DependencyRecorded => "version",
        WebhookEventType::ProvideRejected => "producer",
        WebhookEventType::BranchCut => "graph",
        WebhookEventType::CatalogRecorded => "producer",
        WebhookEventType::Test => "admin",
    };
    links.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// The envelope every format derives from: `id`, `type`, `occurred_at`,
/// `sanshain_version`, `data`, and `links` when a base URL is known.
pub fn envelope(
    event_id: &str,
    event_type: WebhookEventType,
    occurred_at: &str,
    data: &serde_json::Value,
    base_url: Option<&str>,
) -> serde_json::Value {
    let mut envelope = serde_json::json!({
        "id": event_id,
        "type": event_type.as_str(),
        "occurred_at": occurred_at,
        "sanshain_version": env!("CARGO_PKG_VERSION"),
        "data": data,
    });
    if let Some(base) = base_url
        && let Some(object) = envelope.as_object_mut()
    {
        object.insert("links".to_string(), links_for(event_type, data, base));
    }
    envelope
}

/// One sentence stating the fact, for the chat formats.
pub fn summary(event_type: WebhookEventType, data: &serde_json::Value) -> String {
    let s = |name: &str| {
        data.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };
    let b = |name: &str| data.get(name).and_then(|v| v.as_bool()).unwrap_or(false);
    match event_type {
        WebhookEventType::VersionPublished => {
            let what = if b("promoted") {
                "promoted to GA"
            } else if s("stability") == "ga" {
                "published as GA"
            } else {
                "published as a snapshot"
            };
            let compat = data
                .get("compatibility")
                .and_then(|c| c.as_object())
                .map(|c| {
                    format!(
                        " ({} change against {})",
                        c.get("impact").and_then(|v| v.as_str()).unwrap_or("?"),
                        c.get("against").and_then(|v| v.as_str()).unwrap_or("?")
                    )
                })
                .unwrap_or_default();
            format!(
                "Producer {} {what}: {} {} on {}{compat}, by {}",
                s("producer"),
                s("api_type"),
                s("version"),
                s("stream"),
                s("actor")
            )
        }
        WebhookEventType::ProvideRejected => {
            let proposed = data
                .get("proposed_version")
                .and_then(|v| v.as_str())
                .map(|v| format!(" — publish as {v} instead"))
                .unwrap_or_default();
            format!(
                "Provide of {} {} {} rejected ({}){proposed}: {}",
                s("producer"),
                s("api_type"),
                s("version"),
                s("rule"),
                s("message")
            )
        }
        WebhookEventType::VersionDeprecated => {
            let elements = data
                .get("elements")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .map(|e| {
                            format!(
                                "{} {}",
                                e.get("method").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("path").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let whole = if b("document_deprecated") {
                " — the whole document is deprecated"
            } else {
                ""
            };
            format!(
                "Producer {} deprecated in {} {}: {elements}{whole}",
                s("producer"),
                s("api_type"),
                s("version")
            )
        }
        WebhookEventType::DependencyRecorded => {
            let count = data
                .get("endpoints")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let pinned = if b("snapshot_pinned") {
                " (snapshot-pinned)"
            } else {
                ""
            };
            format!(
                "Consumer {} now requires {} {} {} on {}: {count} endpoint(s){pinned}",
                s("consumer"),
                s("producer"),
                s("api_type"),
                s("version"),
                s("stream")
            )
        }
        WebhookEventType::BranchCut => format!(
            "Sanshain-branch {} cut from {} as of {} by {}",
            s("name"),
            s("source"),
            s("as_of"),
            s("actor")
        ),
        WebhookEventType::CatalogRecorded => {
            if b("recorded") {
                format!(
                    "Catalog push of {} recorded {} Peripheral Resource(s){}",
                    s("system"),
                    data.get("resources").and_then(|v| v.as_u64()).unwrap_or(0),
                    if b("trunk") { " on trunk" } else { "" }
                )
            } else {
                format!(
                    "Catalog push of {} recorded nothing: {}",
                    s("system"),
                    s("warning")
                )
            }
        }
        WebhookEventType::Test => format!(
            "Test event from Sanshain for hook {}: {}",
            data.get("hook_id")
                .map(|v| v.to_string())
                .unwrap_or_default(),
            s("message")
        ),
    }
}

/// The POST body in the hook's format: the envelope, a Slack incoming-webhook
/// message, or a Teams Adaptive Card — the same facts in one message plus
/// the deep link, when one is known.
pub fn render_body(format: WebhookFormat, envelope: &serde_json::Value) -> String {
    let event_type = envelope
        .get("type")
        .and_then(|v| v.as_str())
        .and_then(WebhookEventType::parse)
        .unwrap_or(WebhookEventType::Test);
    let data = envelope
        .get("data")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let link = primary_link(event_type, envelope.get("links"));
    let text = summary(event_type, &data);
    let body = match format {
        WebhookFormat::Json => envelope.clone(),
        WebhookFormat::Slack => {
            let text = slack_escape(&text);
            let text = match &link {
                Some(url) => format!(
                    "*{}* — {text} <{url}|Open in Sanshain>",
                    event_type.as_str()
                ),
                None => format!("*{}* — {text}", event_type.as_str()),
            };
            serde_json::json!({ "text": text })
        }
        WebhookFormat::Teams => {
            let mut card = serde_json::json!({
                "type": "AdaptiveCard",
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "version": "1.4",
                "body": [
                    { "type": "TextBlock", "text": event_type.as_str(), "weight": "Bolder", "size": "Medium" },
                    { "type": "TextBlock", "text": teams_escape(&text), "wrap": true },
                ],
            });
            if let Some(url) = &link
                && let Some(object) = card.as_object_mut()
            {
                object.insert(
                    "actions".to_string(),
                    serde_json::json!([
                        { "type": "Action.OpenUrl", "title": "Open in Sanshain", "url": url }
                    ]),
                );
            }
            serde_json::json!({
                "type": "message",
                "attachments": [{
                    "contentType": "application/vnd.microsoft.card.adaptive",
                    "contentUrl": null,
                    "content": card,
                }],
            })
        }
    };
    serde_json::to_string(&body).unwrap_or_default()
}

/// The request one delivery makes: body in the hook's format, the
/// signature over `<t>.<body>`, and the event and delivery headers.
pub fn build_request(
    due: &DueWebhookDelivery,
    base_url: Option<&str>,
    now: DateTime<Utc>,
) -> WebhookRequest {
    let envelope = envelope(
        &due.delivery.event_id,
        due.delivery.event_type,
        &due.delivery.occurred_at,
        &due.data,
        base_url,
    );
    let body = render_body(due.hook.format, &envelope);
    let timestamp = now.timestamp();
    WebhookRequest {
        url: due.hook.url.clone(),
        headers: vec![
            (
                "Sanshain-Event".to_string(),
                due.delivery.event_type.as_str().to_string(),
            ),
            (
                "X-Sanshain-Delivery".to_string(),
                due.delivery.id.to_string(),
            ),
            (
                "Sanshain-Signature".to_string(),
                signature_header(&due.hook.secret, timestamp, &body),
            ),
        ],
        body,
    }
}

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn count(outcome: &'static str) {
    metrics::counter!("sanshain_webhook_deliveries_total", "outcome" => outcome).increment(1);
}

/// One worker pass at `now`: claim what is due, give up on what is past the
/// cutoff, refuse what the allow-list no longer admits, POST the rest, and
/// schedule every failure's next attempt. Each delivery's outcome is
/// recorded on its own, so one slow receiver never holds the others.
#[instrument(skip_all)]
pub async fn deliver_due(
    repo: &impl SpecRepository,
    sender: &impl WebhookSender,
    config: &WebhookDeliveryConfig,
    now: DateTime<Utc>,
) -> Result<DeliveryPassSummary, AppError> {
    let lease_until = iso(super::after(now, super::seconds(config.lease_secs())));
    let due = repo
        .claim_due_webhook_deliveries(&iso(now), &lease_until, config.batch_size)
        .await?;
    let mut summary = DeliveryPassSummary::default();
    if !due.is_empty() {
        let allowed = webhook_allowed_hosts(repo).await?;
        let base_url = resolve_base_url(repo, config.public_base_url.as_deref()).await?;
        for item in &due {
            let outcome = attempt(sender, item, &allowed, base_url.as_deref(), now).await;
            match &outcome {
                WebhookAttemptOutcome::Delivered { .. } => {
                    summary.delivered = summary.delivered.saturating_add(1);
                    count("delivered");
                }
                WebhookAttemptOutcome::Failed { .. } => {
                    summary.failed = summary.failed.saturating_add(1);
                    count("failed");
                }
                WebhookAttemptOutcome::Dead { .. } | WebhookAttemptOutcome::Expired { .. } => {
                    summary.dead = summary.dead.saturating_add(1);
                    count("dead");
                }
            }
            let recorded = repo
                .record_webhook_attempt(item.delivery.id, &iso(now), &outcome, &lease_until)
                .await?;
            if !recorded {
                tracing::warn!(
                    delivery = item.delivery.id,
                    "Webhook delivery outcome discarded: the lease expired and another pass re-claimed the row"
                );
            }
        }
    }
    let pending = repo.count_pending_webhook_deliveries().await?;
    metrics::gauge!("sanshain_webhook_outbox_pending").set(pending as f64);
    Ok(summary)
}

async fn attempt(
    sender: &impl WebhookSender,
    item: &DueWebhookDelivery,
    allowed: &[String],
    base_url: Option<&str>,
    now: DateTime<Utc>,
) -> WebhookAttemptOutcome {
    let parse = |stamp: &str| {
        DateTime::parse_from_rfc3339(stamp)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or(now)
    };
    let cutoff = super::after(
        parse(&item.delivery.occurred_at),
        Duration::hours(CUTOFF_HOURS),
    );
    // A row redelivered by hand gets one more attempt whatever the clock
    // says; everything else past the cutoff is dead without one (a paused
    // hook resumed a day later included). A failure past the cutoff is dead
    // again, so the grant is exactly one attempt.
    let redelivered = item.delivery.redelivered_at.is_some();
    if now >= cutoff && !redelivered {
        return WebhookAttemptOutcome::Expired {
            error: format!("given up: {CUTOFF_HOURS} hours have passed since the event"),
        };
    }
    let next = super::after(now, super::seconds(backoff_secs(item.delivery.attempts)));
    let fail = |code: Option<u16>, error: String| {
        // A retry due after the cutoff would only be marked dead then;
        // saying so now keeps the log honest.
        if next >= cutoff {
            WebhookAttemptOutcome::Dead { code, error }
        } else {
            WebhookAttemptOutcome::Failed {
                code,
                error,
                next_attempt_at: iso(next),
            }
        }
    };
    if !item.hook.allow_list_exempt && !host_allowed(&item.hook.url, allowed) {
        return fail(
            None,
            "the hook's host is not in the allow-list (webhook_allowed_hosts) any more".to_string(),
        );
    }
    let request = build_request(item, base_url, now);
    match sender.send(request).await {
        Ok(response) if (200..300).contains(&response.status) => WebhookAttemptOutcome::Delivered {
            code: response.status,
        },
        Ok(response) => fail(
            Some(response.status),
            format!("the receiver answered HTTP {}", response.status),
        ),
        Err(error) => fail(None, error),
    }
}

/// Remove delivered and dead rows older than the retention window — part of
/// the cleanup cycle. Returns how many deliveries went.
#[instrument(skip_all)]
pub async fn cleanup_webhook_data(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let cutoff = super::before(Utc::now(), Duration::days(RETENTION_DAYS));
    Ok(repo.delete_old_webhook_data(&iso(cutoff)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_starts_at_30s_doubles_and_caps_at_an_hour() {
        assert_eq!(backoff_secs(0), 30);
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(2), 120);
        assert_eq!(backoff_secs(6), 1920);
        assert_eq!(backoff_secs(7), 3600);
        assert_eq!(backoff_secs(40), 3600);
    }

    /// RFC 4231 test case 2: HMAC-SHA256("Jefe", "what do ya want for nothing?").
    #[test]
    fn signature_is_hmac_sha256_over_timestamp_dot_body() {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac.finalize().into_bytes()),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Our framing: the timestamp, a dot, the body.
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(b"whsec_k").unwrap();
        mac.update(b"1700000000.{\"a\":1}");
        let expected = hex::encode(mac.finalize().into_bytes());
        assert_eq!(signature("whsec_k", 1_700_000_000, "{\"a\":1}"), expected);
        assert_eq!(
            signature_header("whsec_k", 1_700_000_000, "{\"a\":1}"),
            format!("t=1700000000,v1={expected}")
        );
    }

    #[test]
    fn origin_keeps_scheme_host_and_port_only() {
        assert_eq!(
            origin_of("https://sanshain.corp/auth/oidc/callback").as_deref(),
            Some("https://sanshain.corp")
        );
        assert_eq!(
            origin_of("http://localhost:3000/auth/oidc/callback").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(origin_of("not a url"), None);
    }

    #[test]
    fn envelope_carries_links_only_with_a_base_url() {
        let data =
            serde_json::json!({"producer": "orders", "api_type": "openapi", "version": "1.0.0"});
        let without = envelope(
            "e1",
            WebhookEventType::VersionPublished,
            "2026-01-01T00:00:00Z",
            &data,
            None,
        );
        assert_eq!(without["id"], "e1");
        assert_eq!(without["type"], "version.published");
        assert_eq!(without["sanshain_version"], env!("CARGO_PKG_VERSION"));
        assert!(without.get("links").is_none());
        let with = envelope(
            "e1",
            WebhookEventType::VersionPublished,
            "2026-01-01T00:00:00Z",
            &data,
            Some("https://s.example"),
        );
        assert_eq!(
            with["links"]["version"],
            "https://s.example/producers.html?service=orders&api_type=openapi&version=1.0.0"
        );
        assert_eq!(with["links"]["graph"], "https://s.example/graph.html");
        let dep = serde_json::json!({"consumer": "web app", "producer": "orders", "api_type": "openapi", "version": "1.0.0"});
        let links = links_for(
            WebhookEventType::DependencyRecorded,
            &dep,
            "https://s.example",
        );
        assert_eq!(
            links["consumer"],
            "https://s.example/consumers.html?name=web+app"
        );
    }

    #[test]
    fn slack_and_teams_bodies_carry_the_summary_and_the_link() {
        let data = serde_json::json!({
            "producer": "orders", "api_type": "openapi", "version": "2.0.0", "stability": "ga",
            "promoted": false, "stream": "trunk", "actor": "alice",
            "compatibility": {"against": "1.4.0", "impact": "major"},
        });
        let env = envelope(
            "e1",
            WebhookEventType::VersionPublished,
            "2026-01-01T00:00:00Z",
            &data,
            Some("https://s.example"),
        );
        let slack: serde_json::Value =
            serde_json::from_str(&render_body(WebhookFormat::Slack, &env)).unwrap();
        let text = slack["text"].as_str().unwrap();
        assert!(text.starts_with("*version.published* — Producer orders published as GA: openapi 2.0.0 on trunk (major change against 1.4.0), by alice"), "{text}");
        assert!(text.contains("<https://s.example/producers.html?service=orders&api_type=openapi&version=2.0.0|Open in Sanshain>"));
        let teams: serde_json::Value =
            serde_json::from_str(&render_body(WebhookFormat::Teams, &env)).unwrap();
        assert_eq!(teams["type"], "message");
        let card = &teams["attachments"][0]["content"];
        assert_eq!(
            teams["attachments"][0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(card["type"], "AdaptiveCard");
        assert_eq!(
            card["body"][1]["text"],
            "Producer orders published as GA: openapi 2.0.0 on trunk (major change against 1.4.0), by alice"
        );
        assert_eq!(card["actions"][0]["type"], "Action.OpenUrl");
        assert_eq!(
            card["actions"][0]["url"],
            "https://s.example/producers.html?service=orders&api_type=openapi&version=2.0.0"
        );
        // No base URL: no link, no actions.
        let env = envelope(
            "e1",
            WebhookEventType::VersionPublished,
            "2026-01-01T00:00:00Z",
            &data,
            None,
        );
        let slack: serde_json::Value =
            serde_json::from_str(&render_body(WebhookFormat::Slack, &env)).unwrap();
        assert!(!slack["text"].as_str().unwrap().contains("Open in Sanshain"));
        let teams: serde_json::Value =
            serde_json::from_str(&render_body(WebhookFormat::Teams, &env)).unwrap();
        assert!(teams["attachments"][0]["content"].get("actions").is_none());
        // json is the envelope itself.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&render_body(WebhookFormat::Json, &env))
                .unwrap(),
            env
        );
    }

    #[test]
    fn chat_bodies_escape_caller_supplied_text_but_not_the_link() {
        let data = serde_json::json!({
            "producer": "<https://evil.example|orders>", "api_type": "openapi", "version": "1.0.0",
            "stability": "ga", "promoted": false, "stream": "dev", "actor": "a&b", "compatibility": null,
        });
        let env = envelope(
            "e1",
            WebhookEventType::VersionPublished,
            "2026-01-01T00:00:00Z",
            &data,
            Some("https://s.example"),
        );
        let slack: serde_json::Value =
            serde_json::from_str(&render_body(WebhookFormat::Slack, &env)).unwrap();
        let text = slack["text"].as_str().unwrap();
        assert!(
            text.contains("&lt;https://evil.example|orders&gt;"),
            "{text}"
        );
        assert!(text.contains("by a&amp;b"), "{text}");
        assert!(!text.contains("<https://evil.example|orders>"), "{text}");
        assert!(text.contains("<https://s.example/producers.html?service=%3Chttps%3A%2F%2Fevil.example%7Corders%3E&api_type=openapi&version=1.0.0|Open in Sanshain>"), "the deep link itself stays a link: {text}");

        let data = serde_json::json!({"name": "[click](https://evil.example) *now*", "source": "trunk", "as_of": "x", "actor": "bob"});
        let env = envelope(
            "e1",
            WebhookEventType::BranchCut,
            "2026-01-01T00:00:00Z",
            &data,
            None,
        );
        let teams: serde_json::Value =
            serde_json::from_str(&render_body(WebhookFormat::Teams, &env)).unwrap();
        let text = teams["attachments"][0]["content"]["body"][1]["text"]
            .as_str()
            .unwrap();
        assert!(
            text.contains("\\[click\\](https://evil.example) \\*now\\*"),
            "{text}"
        );
        assert_eq!(teams_escape("a_b"), "a\\_b");
        assert_eq!(slack_escape("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[test]
    fn the_lease_covers_the_worst_case_pass() {
        let config = WebhookDeliveryConfig::default();
        assert_eq!(config.lease_secs(), 50 * 10 + 60);
        let config = WebhookDeliveryConfig {
            batch_size: 5,
            delivery_timeout_secs: 3,
            ..Default::default()
        };
        assert_eq!(config.lease_secs(), 15 + 60);
    }

    #[test]
    fn summaries_state_the_fact_per_type() {
        let rejected = serde_json::json!({"producer": "orders", "api_type": "proto", "version": "1.0.0", "rule": "immutable_ga", "message": "nope", "proposed_version": "1.0.1"});
        assert_eq!(
            summary(WebhookEventType::ProvideRejected, &rejected),
            "Provide of orders proto 1.0.0 rejected (immutable_ga) — publish as 1.0.1 instead: nope"
        );
        let deprecated = serde_json::json!({"producer": "orders", "api_type": "openapi", "version": "1.1.0", "elements": [{"path": "/a", "method": "GET"}], "document_deprecated": true});
        assert_eq!(
            summary(WebhookEventType::VersionDeprecated, &deprecated),
            "Producer orders deprecated in openapi 1.1.0: GET /a — the whole document is deprecated"
        );
        let dep = serde_json::json!({"consumer": "web", "producer": "orders", "api_type": "openapi", "version": "1.0.0", "stream": "dev", "snapshot_pinned": true, "endpoints": [{}, {}]});
        assert_eq!(
            summary(WebhookEventType::DependencyRecorded, &dep),
            "Consumer web now requires orders openapi 1.0.0 on dev: 2 endpoint(s) (snapshot-pinned)"
        );
        let cut = serde_json::json!({"name": "rel", "source": "trunk", "as_of": "2026-01-01T00:00:00Z", "actor": "bob"});
        assert_eq!(
            summary(WebhookEventType::BranchCut, &cut),
            "Sanshain-branch rel cut from trunk as of 2026-01-01T00:00:00Z by bob"
        );
        let bad = serde_json::json!({"system": "orders", "recorded": false, "warning": "not valid YAML", "resources": 0, "trunk": false});
        assert_eq!(
            summary(WebhookEventType::CatalogRecorded, &bad),
            "Catalog push of orders recorded nothing: not valid YAML"
        );
        let good = serde_json::json!({"system": "orders", "recorded": true, "warning": null, "resources": 2, "trunk": true});
        assert_eq!(
            summary(WebhookEventType::CatalogRecorded, &good),
            "Catalog push of orders recorded 2 Peripheral Resource(s) on trunk"
        );
    }
}
