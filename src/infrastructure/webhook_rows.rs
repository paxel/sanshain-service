//! The row shapes the webhook tables (#47) are read with, and their one
//! mapping into the domain — shared by the SQLite and Postgres repositories.

use crate::domain::models::{
    Webhook, WebhookDelivery, WebhookDeliveryStatus, WebhookEventType, WebhookFormat, WebhookScope,
};
use crate::domain::ports::RepositoryError;

/// `(id, scope_kind, scope_name, url, format, event_types, producer_glob,
/// secret, secret_hint, paused, allow_list_exempt, created_by, created_at)`.
pub(crate) type WebhookRow = (
    i64,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    bool,
    bool,
    String,
    String,
);

/// The column list `WebhookRow` is selected with, prefixed by the table alias.
pub(crate) const WEBHOOK_COLUMNS: &str = "h.id, h.scope_kind, h.scope_name, h.url, h.format, h.event_types, h.producer_glob, h.secret, h.secret_hint, h.paused, h.allow_list_exempt, h.created_by, h.created_at";

/// `(id, hook_id, event_id, event_type, occurred_at, status, attempts,
/// next_attempt_at, last_response_code, last_error, delivered_at,
/// redelivered_at, updated_at)` — generic over the integer width because
/// SQLite decodes INTEGER as `i64` while Postgres decodes INT4 as `i32`.
pub(crate) type DeliveryRow<I> = (
    i64,
    i64,
    String,
    String,
    String,
    String,
    I,
    String,
    Option<I>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// The column list `DeliveryRow` is selected with: the delivery aliased `d`,
/// joined to its event aliased `e`.
pub(crate) const DELIVERY_COLUMNS: &str = "d.id, d.hook_id, d.event_id, e.event_type, e.occurred_at, d.status, d.attempts, d.next_attempt_at, d.last_response_code, d.last_error, d.delivered_at, d.redelivered_at, d.updated_at";

/// The stored spelling of a hook's event-type filter: comma-separated, or
/// `None` for "every type".
pub(crate) fn encode_event_types(types: &[WebhookEventType]) -> Option<String> {
    if types.is_empty() {
        None
    } else {
        Some(
            types
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

fn decode_event_types(stored: Option<&str>) -> Result<Vec<WebhookEventType>, RepositoryError> {
    stored
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            WebhookEventType::parse(s).ok_or_else(|| {
                RepositoryError::Internal(format!("unknown webhook event type '{s}'"))
            })
        })
        .collect()
}

pub(crate) fn webhook_from_row(row: WebhookRow) -> Result<Webhook, RepositoryError> {
    let (
        id,
        scope_kind,
        scope_name,
        url,
        format,
        event_types,
        producer_glob,
        secret,
        secret_hint,
        paused,
        allow_list_exempt,
        created_by,
        created_at,
    ) = row;
    let scope = WebhookScope::from_stored(&scope_kind, scope_name).ok_or_else(|| {
        RepositoryError::Internal(format!("unknown webhook scope '{scope_kind}' on hook {id}"))
    })?;
    let format = WebhookFormat::parse(&format).ok_or_else(|| {
        RepositoryError::Internal(format!("unknown webhook format '{format}' on hook {id}"))
    })?;
    Ok(Webhook {
        id,
        scope,
        url,
        format,
        event_types: decode_event_types(event_types.as_deref())?,
        producer_glob,
        secret_hint,
        paused,
        allow_list_exempt,
        created_by,
        created_at,
        secret,
    })
}

pub(crate) fn delivery_from_row<I: Into<i64>>(
    row: DeliveryRow<I>,
) -> Result<WebhookDelivery, RepositoryError> {
    let (
        id,
        hook_id,
        event_id,
        event_type,
        occurred_at,
        status,
        attempts,
        next_attempt_at,
        last_response_code,
        last_error,
        delivered_at,
        redelivered_at,
        updated_at,
    ) = row;
    let attempts: i64 = attempts.into();
    let event_type = WebhookEventType::parse(&event_type).ok_or_else(|| {
        RepositoryError::Internal(format!("unknown webhook event type '{event_type}'"))
    })?;
    let status = WebhookDeliveryStatus::parse(&status).ok_or_else(|| {
        RepositoryError::Internal(format!(
            "unknown delivery status '{status}' on delivery {id}"
        ))
    })?;
    Ok(WebhookDelivery {
        id,
        hook_id,
        event_id,
        event_type,
        occurred_at,
        status,
        attempts: attempts.max(0) as u32,
        next_attempt_at,
        last_response_code: last_response_code.map(|c| c.into().clamp(0, u16::MAX as i64) as u16),
        last_error,
        delivered_at,
        redelivered_at,
        updated_at,
    })
}

/// The JSON text an event's `data` is stored as.
pub(crate) fn data_json(data: &serde_json::Value) -> Result<String, RepositoryError> {
    serde_json::to_string(data).map_err(|e| RepositoryError::Internal(e.to_string()))
}

pub(crate) fn data_from_json(text: &str) -> Result<serde_json::Value, RepositoryError> {
    serde_json::from_str(text).map_err(|e| RepositoryError::Internal(e.to_string()))
}
