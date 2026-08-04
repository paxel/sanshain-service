use crate::AppState;
use crate::application::services;
use crate::domain::models::{AppError, LogEntry};
use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

async fn resolve_dev_user(state: &AppState) -> Option<crate::domain::models::User> {
    if let Some(u) = &state.dev_user {
        return Some(u.clone());
    }
    services::ensure_dev_user(&state.repo).await.ok()
}

/// The directory groups a caller is currently in.
///
/// Consulted on every authorisation check rather than captured at login, so a
/// promotion or demotion in the directory takes effect without the user signing
/// in again. The cache is checked first precisely so the common path builds no
/// directory connection at all.
///
/// Returns nothing when the instance does not use the directory for
/// authentication — there is then no directory to ask.
async fn directory_groups_for(state: &AppState, username: &str) -> Vec<String> {
    if let Some(cached) = state.directory_roles.cached(username).await {
        return cached.as_ref().clone();
    }

    if !matches!(
        services::get_auth_mode(&state.repo).await,
        Ok(crate::domain::models::AuthMode::Ldap)
    ) {
        return Vec::new();
    }

    let Ok(Some(config)) = services::get_ldap_config(&state.repo).await else {
        return Vec::new();
    };
    let provider = crate::infrastructure::ldap_provider::LdapAuthProvider::new(config);
    state
        .directory_roles
        .groups_for(&provider, username)
        .await
        .as_ref()
        .clone()
}

/// Attach the authenticated caller to the request.
///
/// Both the stored record and the resolved [`Actor`] go in: handlers that only
/// need an identity keep taking the record, while authorisation reads the
/// Actor. Every entry point goes through here so no path can supply one without
/// the other.
///
/// [`Actor`]: crate::domain::permissions::Actor
/// Returns `Err` when the caller's roles cannot be resolved.
///
/// A request whose authorisation could not be determined must not proceed as if
/// the answer were "no permissions": that is indistinguishable from a genuine
/// refusal, so a transient database fault would look like a policy decision.
async fn attach_caller(
    req: &mut Request,
    state: &AppState,
    user: crate::domain::models::User,
) -> Result<(), StatusCode> {
    let directory_groups = directory_groups_for(state, &user.username).await;
    let actor = crate::application::authz::resolve_actor(
        &state.repo,
        &user,
        &state.root_users,
        &directory_groups,
    )
    .await
    .map_err(|e| {
        tracing::error!(
            "Could not resolve roles for user {}, refusing the request: {}",
            user.id,
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    req.extensions_mut().insert(actor);
    req.extensions_mut().insert(user);
    Ok(())
}

pub async fn authenticated_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    if services::get_dev_mode(&state.repo).await.unwrap_or(false)
        && let Some(dev_user) = resolve_dev_user(&state).await
    {
        let mut req = req;
        attach_caller(&mut req, &state, dev_user).await?;
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
                    attach_caller(&mut req, &state, user).await?;
                    Ok(next.run(req).await)
                }
                _ => Err(StatusCode::UNAUTHORIZED),
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// What a route requires of its caller.
///
/// Declared per route and attached as a request extension, so `permission_auth`
/// enforces one rule for every route rather than each route growing its own
/// check. Making it an explicit value — rather than the absence of one meaning
/// "administrator" — is what lets a build-time check see that a route stated its
/// requirement at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteGuard {
    /// Any signed-in caller. Used by the read-only listings the dashboard needs
    /// before it knows who is looking.
    Authenticated,
    /// The caller must hold this permission instance-wide.
    Global(crate::domain::permissions::Permission),
    /// The caller must hold this permission instance-wide, **or** maintain the
    /// Producer named by the route's `name` path parameter.
    Producer(crate::domain::permissions::Permission),
}

/// The Producer a Producer-scoped route is acting on.
///
/// Read from the matched route's path parameters rather than parsed out of the
/// URI, so percent-encoding and the exact route shape stay the router's problem.
async fn producer_from_path(parts: &mut axum::http::request::Parts) -> Option<String> {
    use axum::extract::FromRequestParts;
    let params = axum::extract::RawPathParams::from_request_parts(parts, &())
        .await
        .ok()?;
    params
        .iter()
        .find(|(key, _)| *key == "name")
        .map(|(_, value)| value.to_string())
}

/// Build the middleware enforcing `guard` for one route.
///
/// The guard is captured here rather than attached as a request extension, so a
/// route carries exactly one layer and the requirement is visible on the same
/// line as the route it protects.
/// The future the guard middleware returns.
///
/// Boxed so `require`'s return type can be written down at all: the layer's type
/// mentions the closure's future, and an `async fn`'s future is unnameable.
type GuardFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, Response>> + Send>>;

/// The middleware layer `require` produces.
type GuardLayer<F> = axum::middleware::FromFnLayer<F, AppState, (State<AppState>, Request)>;

pub fn require(
    state: AppState,
    guard: RouteGuard,
) -> GuardLayer<impl Fn(State<AppState>, Request, Next) -> GuardFuture + Clone> {
    axum::middleware::from_fn_with_state(
        state,
        move |State(state): State<AppState>, req: Request, next: Next| {
            Box::pin(permission_auth(state, guard, req, next)) as GuardFuture
        },
    )
}

/// Authenticate the caller and enforce the route's declared requirement.
///
/// Refusals surface as an [`AppError`] response, not a bare status: the 403 an
/// application-layer check produces can carry a reason
/// (`ForbiddenWithReason`), and flattening it here would silently break the
/// promise that refusals are instructive.
pub async fn permission_auth(
    state: AppState,
    guard: RouteGuard,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let user = if services::get_dev_mode(&state.repo).await.unwrap_or(false)
        && let Some(dev_user) = resolve_dev_user(&state).await
    {
        dev_user
    } else {
        let token = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|t| t.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::Unauthorized.into_response())?;
        match services::validate_session(&state.repo, token).await {
            Ok(Some((user, _session))) => user,
            _ => return Err(AppError::Unauthorized.into_response()),
        }
    };

    let mut req = req;
    attach_caller(&mut req, &state, user)
        .await
        .map_err(IntoResponse::into_response)?;

    let actor = req
        .extensions()
        .get::<crate::domain::permissions::Actor>()
        .cloned()
        .ok_or_else(|| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    match guard {
        RouteGuard::Authenticated => {}
        RouteGuard::Global(permission) => {
            if !actor.has_permission(permission) {
                return Err(AppError::Forbidden.into_response());
            }
        }
        RouteGuard::Producer(permission) => {
            if !actor.has_permission(permission) {
                let (mut parts, body) = req.into_parts();
                let producer = producer_from_path(&mut parts).await;
                req = Request::from_parts(parts, body);

                // Delegated rather than re-implemented: the application layer's
                // check also requires the maintainer *bundle* to include the
                // permission. Duplicating the logic here once omitted that, so a
                // route declared with a permission outside the bundle would have
                // over-granted to maintainers.
                let producer = producer.ok_or_else(|| AppError::Forbidden.into_response())?;
                crate::application::authz::require_producer_permission(
                    &state.repo,
                    &actor,
                    permission,
                    &producer,
                )
                .await
                .map_err(IntoResponse::into_response)?;
            }
        }
    }

    Ok(next.run(req).await.into_response())
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

    // API tokens must be supplied via the `Authorization` header only. Accepting
    // them from a `?token=` query parameter would leak long-lived credentials
    // through server/proxy logs, browser history, and referrers.
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    if let Some(token) = auth_header {
        let token = token.strip_prefix("Bearer ").unwrap_or(token);

        if let Ok(Some((user, _))) = services::validate_session(&state.repo, token).await {
            let mut req = req;
            attach_caller(&mut req, &state, user)
                .await
                .map_err(IntoResponse::into_response)?;
            return Ok(next.run(req).await);
        }

        if let Ok(Some(user)) = services::validate_api_token(&state.repo, token).await {
            let mut req = req;
            attach_caller(&mut req, &state, user)
                .await
                .map_err(IntoResponse::into_response)?;
            return Ok(next.run(req).await);
        }

        // Token was provided but invalid
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }

    if services::get_dev_mode(&state.repo).await.unwrap_or(false)
        && let Some(dev_user) = resolve_dev_user(&state).await
    {
        let mut req = req;
        attach_caller(&mut req, &state, dev_user)
            .await
            .map_err(IntoResponse::into_response)?;
        return Ok(next.run(req).await);
    }

    Err(StatusCode::FORBIDDEN.into_response())
}

pub struct LogVisitor<'a> {
    pub message: &'a mut String,
    /// Captured from an event field literally named `service`, if present
    /// (e.g. `tracing::info!(service = producername, version = version, "...")`
    /// on the provide/require paths). Lets the observability log viewer show
    /// which service/version a line refers to.
    pub service: &'a mut Option<String>,
    pub version: &'a mut Option<String>,
}

impl<'a> tracing::field::Visit for LogVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let formatted = || {
            let mut s = format!("{:?}", value);
            if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
                s = s[1..s.len() - 1].to_string();
            }
            s
        };
        match field.name() {
            "message" => *self.message = formatted(),
            "service" => *self.service = Some(formatted()),
            "version" => *self.version = Some(formatted()),
            _ => {}
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => *self.message = value.to_string(),
            "service" => *self.service = Some(value.to_string()),
            "version" => *self.version = Some(value.to_string()),
            _ => {}
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
        let mut service = None;
        let mut version = None;
        let mut visitor = LogVisitor {
            message: &mut message,
            service: &mut service,
            version: &mut version,
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
            service,
            version,
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

    if services::get_dev_mode(&state.repo).await.unwrap_or(false) {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path();
    if path == "/auth/login" || path == "/auth/register" {
        return Ok(next.run(req).await);
    }
    // The public validator is stateless and anonymous: there is no session to
    // ride and no state to change, so there is nothing for CSRF to protect.
    if path == "/validate" {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    fn test_layer() -> (LogCaptureLayer, Arc<std::sync::Mutex<VecDeque<LogEntry>>>) {
        let info_buffer = Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let layer = LogCaptureLayer {
            error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            warn_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            info_buffer: info_buffer.clone(),
            debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            business_logic_debug: Arc::new(AtomicBool::new(false)),
            admin_user_debug: Arc::new(AtomicBool::new(false)),
        };
        (layer, info_buffer)
    }

    // An event carrying `service`/`version` fields (as used on the provide/require
    // paths) must have them captured on the resulting LogEntry, not silently
    // dropped like every field used to be except "message".
    #[test]
    fn captures_service_and_version_fields_from_an_event() {
        let (layer, info_buffer) = test_layer();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(service = "svc-a", version = "1.2.0", "did a thing");
        });

        let buf = info_buffer.lock().unwrap();
        assert_eq!(buf.len(), 1);
        let entry = &buf[0];
        assert_eq!(entry.message, "did a thing");
        assert_eq!(entry.service.as_deref(), Some("svc-a"));
        assert_eq!(entry.version.as_deref(), Some("1.2.0"));
    }

    // An event with no service/version fields must leave them as None, not
    // fabricate a value or drop the entry.
    #[test]
    fn leaves_service_and_version_none_when_absent() {
        let (layer, info_buffer) = test_layer();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("no context here");
        });

        let buf = info_buffer.lock().unwrap();
        assert_eq!(buf.len(), 1);
        let entry = &buf[0];
        assert_eq!(entry.message, "no context here");
        assert_eq!(entry.service, None);
        assert_eq!(entry.version, None);
    }
}
