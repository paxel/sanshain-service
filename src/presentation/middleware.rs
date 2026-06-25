use crate::AppState;
use crate::application::services;
use crate::domain::models::LogEntry;
use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::IntoResponse,
};
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub async fn authenticated_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    if services::get_dev_mode(&state.repo).await.unwrap_or(false) {
        return Ok(next.run(req).await);
    }
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(token) if token.starts_with("Bearer ") => {
            let token = &token[7..];
            match services::validate_session(&state.repo, token).await {
                Ok(Some((user, _session))) => {
                    let mut req = req;
                    req.extensions_mut().insert(user);
                    Ok(next.run(req).await)
                }
                _ => Err(StatusCode::UNAUTHORIZED),
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub async fn admin_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    if services::get_dev_mode(&state.repo).await.unwrap_or(false) {
        return Ok(next.run(req).await);
    }
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(token) if token.starts_with("Bearer ") => {
            let token = &token[7..];
            match services::validate_session(&state.repo, token).await {
                Ok(Some((user, _session))) => {
                    if user.is_admin {
                        let mut req = req;
                        req.extensions_mut().insert(user);
                        Ok(next.run(req).await)
                    } else {
                        Err(StatusCode::FORBIDDEN)
                    }
                }
                _ => Err(StatusCode::UNAUTHORIZED),
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub async fn api_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<axum::response::Response, axum::response::Response> {
    if let Ok(crate::domain::models::AuthMode::Disabled) =
        services::get_auth_mode(&state.repo).await
    {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Service is in Maintenance Mode / Disabled",
        )
            .into_response());
    }

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let query_token =
        axum::extract::Query::<std::collections::HashMap<String, String>>::try_from_uri(req.uri())
            .ok()
            .and_then(|q| q.get("token").cloned());

    if let Some(token) = auth_header.or(query_token.as_deref()) {
        let token = token.strip_prefix("Bearer ").unwrap_or(token);

        if let Ok(Some((user, _))) = services::validate_session(&state.repo, token).await {
            let mut req = req;
            req.extensions_mut().insert(user);
            return Ok(next.run(req).await);
        }

        if let Ok(Some(user)) = services::validate_api_token(&state.repo, token).await {
            let mut req = req;
            req.extensions_mut().insert(user);
            return Ok(next.run(req).await);
        }

        // Token was provided but invalid
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }

    if services::get_dev_mode(&state.repo).await.unwrap_or(false) {
        return Ok(next.run(req).await);
    }

    Err(StatusCode::FORBIDDEN.into_response())
}

pub struct LogVisitor<'a> {
    pub message: &'a mut String,
}

impl<'a> tracing::field::Visit for LogVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.message = format!("{:?}", value);
            if self.message.starts_with('"')
                && self.message.ends_with('"')
                && self.message.len() >= 2
            {
                *self.message = self.message[1..self.message.len() - 1].to_string();
            }
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.message = value.to_string();
        }
    }
    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        if field.name() == "message" {
            *self.message = value.to_string();
        }
    }
}

pub struct LogCaptureLayer {
    pub error_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub warn_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub info_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub debug_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub business_logic_debug: Arc<AtomicBool>,
    pub admin_user_debug: Arc<AtomicBool>,
}

impl<S> tracing_subscriber::Layer<S> for LogCaptureLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();

        if *level == tracing::Level::DEBUG {
            let is_business = target.starts_with("sanshain_service::application")
                || target.starts_with("sanshain_service::domain")
                || target.starts_with("sanshain_service::infrastructure");
            if is_business {
                if !self.business_logic_debug.load(Ordering::Relaxed) {
                    return;
                }
            } else if !self.admin_user_debug.load(Ordering::Relaxed) {
                return;
            }
        }

        let mut message = String::new();
        let mut visitor = LogVisitor {
            message: &mut message,
        };
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: Utc::now().to_rfc3339(),
            level: level.to_string(),
            target: target.to_string(),
            message: if message.is_empty() {
                "[no message]".to_string()
            } else {
                message
            },
        };

        let (buf_to_use, max_size) = match *level {
            tracing::Level::ERROR => (&self.error_buffer, 100),
            tracing::Level::WARN => (&self.warn_buffer, 100),
            tracing::Level::INFO => (&self.info_buffer, 100),
            _ => (&self.debug_buffer, 100),
        };

        let mut buf = buf_to_use.lock().unwrap_or_else(|e| e.into_inner());
        if buf.len() >= max_size {
            buf.pop_front();
        }
        buf.push_back(entry);
    }
}

pub async fn validate_csrf(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    if req.method() == axum::http::Method::GET || req.method() == axum::http::Method::HEAD {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path();
    if path == "/auth/login" || path == "/auth/register" {
        return Ok(next.run(req).await);
    }

    let csrf_header = req
        .headers()
        .get("X-CSRF-Token")
        .and_then(|h| h.to_str().ok());

    if let Some(token) = csrf_header {
        let tokens = state.csrf_tokens.read().await;
        if let Some(expiry) = tokens.get(token)
            && *expiry > chrono::Utc::now()
        {
            return Ok(next.run(req).await);
        }
    }

    // Fallback for non-browser API clients that authenticate with a Bearer token.
    // CSRF only protects against ambient-credential (cookie) requests forged by a
    // browser; a cross-site request cannot set a custom `Authorization: Bearer`
    // header, so requiring the `Bearer ` prefix here is a safe exemption. The token
    // itself is still validated by the per-route auth middleware.
    let has_bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(|v| v.starts_with("Bearer "))
        .unwrap_or(false);
    if has_bearer {
        return Ok(next.run(req).await);
    }

    Err(StatusCode::FORBIDDEN)
}
