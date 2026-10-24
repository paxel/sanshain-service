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

/// The answer a guard gives instead of running the route.
///
/// Boxed because an axum `Response` is 128 bytes, and a `Result` carrying one
/// unboxed makes even its `Ok(())` that large (`clippy::result_large_err`).
/// A refusal is the rare path, so the allocation lands there.
#[derive(Debug)]
pub struct Refusal(Box<Response>);

impl From<Response> for Refusal {
    fn from(response: Response) -> Self {
        Self(Box::new(response))
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        *self.0
    }
}

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
pub(crate) async fn directory_groups_for(state: &AppState, username: &str) -> Vec<String> {
    if let Some(cached) = state.directory_roles.cached(username).await {
        return cached.as_ref().clone();
    }

    // The sign-in source, not the mode: during Off the remembered LDAP still
    // confers its administrators' roles.
    if !matches!(
        services::sign_in_source(&state.repo).await,
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
    if let Some(slot) = req.extensions().get::<CallerSlot>() {
        slot.set(&actor.username);
    }
    req.extensions_mut().insert(actor);
    req.extensions_mut().insert(user);
    Ok(())
}

/// Off and Dev admit only holders of `manage_auth_config` to the web UI; a
/// session anyone else opened before stops working.
fn refuse_unless_admin_may_sign_in<'a>(
    state: &'a AppState,
    req: &Request,
) -> impl std::future::Future<Output = Result<(), Refusal>> + Send + 'a {
    // Read before the await: a request is not `Sync`, so no reference to it
    // may be held across one.
    let may = req
        .extensions()
        .get::<crate::domain::permissions::Actor>()
        .is_some_and(|actor| {
            actor.has_permission(crate::domain::permissions::Permission::ManageAuthConfig)
        });
    admins_only_gate(state, may)
}

async fn admins_only_gate(state: &AppState, may: bool) -> Result<(), Refusal> {
    let mode = services::get_auth_mode(&state.repo)
        .await
        .map_err(IntoResponse::into_response)?;
    if may || !services::admins_only(&mode) {
        Ok(())
    } else {
        Err(services::admins_only_refusal(&mode).into_response().into())
    }
}

/// The message of an error answer, put on the response by
/// `AppError::into_response` for [`log_errors`] to log with its request.
#[derive(Clone, Debug)]
pub struct ErrorMessage(pub String);

/// Where the authentication middleware leaves the caller's name for
/// [`log_errors`], which runs outside it and so never sees the request
/// extensions it adds.
#[derive(Clone, Default)]
pub struct CallerSlot(Arc<std::sync::Mutex<Option<String>>>);

impl CallerSlot {
    pub fn set(&self, name: &str) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(name.to_string());
        }
    }

    fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|slot| slot.clone())
    }
}

/// Logs every error answer with what an operator needs to trace it: method,
/// path and caller, e.g. `Client error: Forbidden (POST /admin/settings/cache,
/// user dev-1)`. The query string stays out — it may carry names a log should
/// not — and so does any credential.
pub async fn log_errors(mut req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let slot = CallerSlot::default();
    req.extensions_mut().insert(slot.clone());
    let response = next.run(req).await;
    if let Some(ErrorMessage(message)) = response.extensions().get::<ErrorMessage>() {
        let caller = slot
            .get()
            .map_or_else(|| "anonymous".to_string(), |name| format!("user {name}"));
        let status = response.status();
        if status.is_server_error() {
            tracing::error!("Server error: {message} ({method} {path}, {caller})");
        } else if status.is_client_error() {
            tracing::warn!("Client error: {message} ({method} {path}, {caller})");
        }
    }
    response
}

pub async fn authenticated_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Refusal> {
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
                    attach_caller(&mut req, &state, user)
                        .await
                        .map_err(IntoResponse::into_response)?;
                    refuse_unless_admin_may_sign_in(&state, &req).await?;
                    Ok(next.run(req).await)
                }
                _ => Err(StatusCode::UNAUTHORIZED.into_response().into()),
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED.into_response().into()),
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
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, Refusal>> + Send>>;

/// The middleware layer `require` produces.
type GuardLayer<F> = axum::middleware::FromFnLayer<F, AppState, (State<AppState>, Request)>;

pub fn require(
    state: AppState,
    guard: RouteGuard,
) -> GuardLayer<impl Fn(State<AppState>, Request, Next) -> GuardFuture + Clone> {
    axum::middleware::from_fn_with_state(
        state,
        move |State(state): State<AppState>, req: Request, next: Next| {
            Box::pin(permission_auth_with(
                state,
                guard,
                AdminCredentials::SessionOnly,
                req,
                next,
            )) as GuardFuture
        },
    )
}

/// Like [`require`], additionally admitting a `full` API token as its owner.
///
/// For the administrative routes automation calls on a schedule (#50: the
/// backup CronJob). A `read_only` token is refused with `403`, the same as on
/// a catalog-changing API route; the permission check is unchanged.
pub fn require_with_api_tokens(
    state: AppState,
    guard: RouteGuard,
) -> GuardLayer<impl Fn(State<AppState>, Request, Next) -> GuardFuture + Clone> {
    axum::middleware::from_fn_with_state(
        state,
        move |State(state): State<AppState>, req: Request, next: Next| {
            Box::pin(permission_auth_with(
                state,
                guard,
                AdminCredentials::SessionOrApiToken,
                req,
                next,
            )) as GuardFuture
        },
    )
}

/// What a route behind [`require`] accepts as proof of identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdminCredentials {
    /// A session token only — the dashboard's routes.
    SessionOnly,
    /// A session token, or a `full` API token acting as its owner.
    SessionOrApiToken,
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
) -> Result<Response, Refusal> {
    permission_auth_with(state, guard, AdminCredentials::SessionOnly, req, next).await
}

async fn permission_auth_with(
    state: AppState,
    guard: RouteGuard,
    credentials: AdminCredentials,
    req: Request,
    next: Next,
) -> Result<Response, Refusal> {
    // Dev mode opens the API, not the administration: the routes behind a
    // guard always need a signed-in caller.
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|t| t.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized.into_response())?;
    let user = match services::validate_session(&state.repo, token).await {
        Ok(Some((user, _session))) => user,
        _ if credentials == AdminCredentials::SessionOrApiToken => {
            match services::validate_api_token(&state.repo, token).await {
                Ok(Some(identity)) => {
                    if identity.scope == crate::domain::models::ApiTokenScope::ReadOnly {
                        return Err(read_only_admin_token_refusal().into_response().into());
                    }
                    identity.user
                }
                _ => return Err(AppError::Unauthorized.into_response().into()),
            }
        }
        _ => return Err(AppError::Unauthorized.into_response().into()),
    };

    let mut req = req;
    attach_caller(&mut req, &state, user)
        .await
        .map_err(IntoResponse::into_response)?;
    refuse_unless_admin_may_sign_in(&state, &req).await?;

    let actor = req
        .extensions()
        .get::<crate::domain::permissions::Actor>()
        .cloned()
        .ok_or_else(|| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    match guard {
        RouteGuard::Authenticated => {}
        RouteGuard::Global(permission) => {
            if !actor.has_permission(permission) {
                return Err(AppError::Forbidden.into_response().into());
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

/// What a route behind the API guard does to the catalog.
///
/// Declared per route rather than checked per handler, so a read-only token's
/// refusal is one rule at the router and not an `if` every writer must
/// remember.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiAccess {
    /// Reads only — every API token may pass.
    Read,
    /// Changes the catalog — a `read_only` token is refused with `403`.
    Write,
}

/// Guard for the Producer/Consumer API routes that only read.
pub async fn api_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Refusal> {
    authenticate_api(state, ApiAccess::Read, req, next).await
}

/// Guard for the Producer/Consumer API routes that change the catalog:
/// provide (all types), retire, require, catalog push. Same authentication
/// as [`api_auth`], plus the refusal of `read_only` tokens.
pub async fn api_auth_write(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Refusal> {
    authenticate_api(state, ApiAccess::Write, req, next).await
}

/// The refusal a read-only token gets on an administrative route that admits
/// API tokens.
fn read_only_admin_token_refusal() -> AppError {
    AppError::ForbiddenWithReason(format!(
        "this API token has scope '{}' and may not call administrative routes; use a token with scope '{}' of an account holding the permission",
        crate::domain::models::ApiTokenScope::ReadOnly.as_str(),
        crate::domain::models::ApiTokenScope::Full.as_str(),
    ))
}

/// The refusal a read-only token gets on a write route.
pub fn read_only_token_refusal() -> AppError {
    AppError::ForbiddenWithReason(format!(
        "this API token has scope '{}' and may not change the catalog; create a token with scope '{}' for provide, require and catalog pushes",
        crate::domain::models::ApiTokenScope::ReadOnly.as_str(),
        crate::domain::models::ApiTokenScope::Full.as_str(),
    ))
}

async fn authenticate_api(
    state: AppState,
    access: ApiAccess,
    req: Request,
    next: Next,
) -> Result<Response, Refusal> {
    if let Ok(crate::domain::models::AuthMode::Disabled) =
        services::get_auth_mode(&state.repo).await
    {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Service is in Maintenance Mode / Disabled",
        )
            .into_response()
            .into());
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

        if let Ok(Some(identity)) = services::validate_api_token(&state.repo, token).await {
            if access == ApiAccess::Write
                && identity.scope == crate::domain::models::ApiTokenScope::ReadOnly
            {
                return Err(read_only_token_refusal().into_response().into());
            }
            let mut req = req;
            attach_caller(&mut req, &state, identity.user)
                .await
                .map_err(IntoResponse::into_response)?;
            return Ok(next.run(req).await);
        }

        // Token was provided but invalid
        return Err(StatusCode::UNAUTHORIZED.into_response().into());
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

    Err(StatusCode::FORBIDDEN.into_response().into())
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
    /// Entries kept per level. `LOG_BUFFER_SIZE` configures this; it used to be
    /// hardcoded to 100 here while `main` sized the buffers from the setting,
    /// so the documented knob moved the allocation and nothing else.
    pub max_size: usize,
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

        let buf_to_use = match *level {
            tracing::Level::ERROR => &self.error_buffer,
            tracing::Level::WARN => &self.warn_buffer,
            tracing::Level::INFO => &self.info_buffer,
            _ => &self.debug_buffer,
        };

        let mut buf = buf_to_use.lock().unwrap_or_else(|e| e.into_inner());
        // A zero-sized buffer would make the admin log view permanently empty
        // and is far more likely a typo than an intent, so one entry is kept.
        let max_size = self.max_size.max(1);
        while buf.len() >= max_size {
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
    // The MCP endpoint accepts API tokens only — never a session or cookie —
    // so there is no ambient credential a cross-site request could ride; its
    // own guard answers a missing token with the 401 the protocol expects.
    if path == "/mcp" {
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
        test_layer_sized(100)
    }

    fn test_layer_sized(
        max_size: usize,
    ) -> (LogCaptureLayer, Arc<std::sync::Mutex<VecDeque<LogEntry>>>) {
        let info_buffer = Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let layer = LogCaptureLayer {
            max_size,
            error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            warn_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            info_buffer: info_buffer.clone(),
            debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            business_logic_debug: Arc::new(AtomicBool::new(false)),
            admin_user_debug: Arc::new(AtomicBool::new(false)),
        };
        (layer, info_buffer)
    }

    // Boxing a refusal must not change the answer: status and headers come back
    // exactly as the guard built them.
    #[test]
    fn a_refusal_answers_with_the_response_it_was_made_from() {
        let response = (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();

        let answer = Refusal::from(response).into_response();

        assert_eq!(answer.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(answer.headers()[header::WWW_AUTHENTICATE], "Bearer");
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

    // LOG_BUFFER_SIZE is documented as the retention knob; before this it only
    // sized the initial allocation while the trim stayed at a hardcoded 100.
    #[test]
    fn keeps_at_most_the_configured_number_of_entries() {
        let (layer, info_buffer) = test_layer_sized(3);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            for i in 0..10 {
                tracing::info!("entry {}", i);
            }
        });

        let buf = info_buffer.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(buf.len(), 3, "buffer must hold exactly the configured size");
        // The newest survive; the oldest are evicted.
        assert_eq!(buf[0].message, "entry 7");
        assert_eq!(buf[2].message, "entry 9");
    }

    // A zero is far more likely a typo than a request for a permanently empty
    // admin log view, so one entry is still kept.
    #[test]
    fn a_zero_size_still_keeps_one_entry() {
        let (layer, info_buffer) = test_layer_sized(0);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("first");
            tracing::info!("second");
        });

        let buf = info_buffer.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(buf.len(), 1);
        assert_eq!(buf[0].message, "second");
    }
}
