//! Authentication — credentials, server-side sessions, and the panel gate
//! (ADR-0013, spec #127).
//!
//! A [`Panel`] is gated by default. The shipped [`PasswordAuth`]
//! verifies Argon2id PHC hashes against the [`AdminUser`] model, a session
//! cookie issued by Topcoat's token transport identifies one server-side
//! [`AuthSession`] row, and the resolved [`CurrentUser`] travels in request
//! `Cx` for pages, shards, and app code. An app with its own user table
//! implements [`Authenticator`] and swaps it in with
//! [`Panel::auth`](crate::Panel::auth); [`Auth::disabled`] is the explicit,
//! greppable opt-out for public demos.
//!
//! Sessions are always the framework's: the shipped [`AuthSession`] table maps
//! a token hash to a user id, so an app registers it whatever authenticator it
//! uses. A custom user model does not have to be an `AdminUser`.

use std::{future::Future, pin::Pin, time::Duration};

use jiff::Timestamp;
use toasty::Db;
use topcoat::{
    context::{Cx, app_context, try_app_context, try_request_context},
    router::{
        Body, Layer, LayerFuture, Next, Path, PathBuf, RouteFuture,
        error::{forbidden, redirect, unauthorized},
        request::{method, original_headers, original_method, uri},
        response::IntoResponse,
    },
    session::{self, RouterBuilderSessionExt, SessionConfig, TokenHash},
    view::{BoxView, ViewExt},
};
use uuid::Uuid;

use crate::panel::{LoginHint, Panel, PanelPrefix, route_path};

/// How long a session stays valid: seven days, fixed (ADR-0013).
pub const SESSION_LIFETIME: Duration = Duration::from_hours(24 * 7);

/// Max body the login route accepts: 64 KiB.
///
/// A credential form submits an email, a password, a `next` and a CSRF token,
/// all short strings; the panel's 10 MiB form cap is for multipart uploads,
/// which the login route never carries. 64 KiB leaves room for the fields plus
/// percent-encoding growth while keeping the one unauthenticated POST route
/// from buffering a megabyte-scale body.
pub(crate) const MAX_LOGIN_BYTES: usize = 64 * 1024;

/// Form field carrying the login identifier (the shipped default reads it as
/// an email address).
pub const LOGIN_FIELD: &str = "email";
/// Form field carrying the password.
pub const PASSWORD_FIELD: &str = "password";
/// Hidden form field carrying the validated post-login destination.
pub const NEXT_FIELD: &str = "next";

/// The one error every failed login renders, so accounts cannot be
/// enumerated and panel membership stays private (ADR-0013).
const GENERIC_ERROR: &str = "Invalid email or password.";

/// What a failed login attempt renders when the database behind it could not
/// answer.
///
/// [`GENERIC_ERROR`] is deliberately non-specific about *why* credentials were
/// refused, so reusing it for an outage would tell a user their password was
/// wrong while the database was down — the one place they would retry
/// uselessly. This copy names the machinery that is down instead, and like the
/// generic one it never carries driver text: that goes to the log.
const UNAVAILABLE_ERROR: &str = "Sign-in is unavailable right now. Try again shortly.";

/// A dummy Argon2id PHC string verified against when the account does not
/// exist, so unknown emails pay the same work as known ones (ADR-0013).
/// Generated with `Argon2::default()` parameters (`m=19456,t=2,p=1`).
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$h3oXdPBVwhcgZ1OTO/PuzQ$zLrHLgIkwhqu4ZlLTfSyB8mPuL6mAtaswv/eXJ5ADO8";

/// The shipped credential model (ADR-0013): unique email, Argon2id PHC hash,
/// display name, active flag, and an optional tenant.
///
/// Register it (plus [`AuthSession`]) in the app's `Db` model list, seed one
/// row, and the default [`PasswordAuth`] works with no further wiring:
/// `toasty::models!(…, tablo_core::auth::AdminUser, tablo_core::auth::AuthSession)`.
#[derive(Debug, Clone, toasty::Model)]
pub struct AdminUser {
    #[key]
    #[auto]
    pub id: Uuid,
    #[unique]
    pub email: String,
    /// Argon2id password hash in PHC string format.
    pub password_hash: String,
    pub display_name: String,
    /// `false` denies login and panel access immediately.
    pub active: bool,
    pub tenant_id: Option<Uuid>,
    pub created_at: Timestamp,
}

/// The shipped server-side session record (ADR-0013): the SHA-256 hash of the
/// client token, the user it authenticates (opaque id), and its expiry.
///
/// The raw token is never stored; a leaked session table contains nothing a
/// client could present. Register this model alongside the app's user model.
#[derive(Debug, Clone, toasty::Model)]
pub struct AuthSession {
    /// Hex-encoded SHA-256 of the session token (the raw token stays client-side).
    #[key]
    pub token_hash: String,
    /// [`CurrentUser::id`] of the authenticated user.
    #[index]
    pub user_id: String,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
}

/// The erased identity resolution places in request `Cx` (ADR-0013).
///
/// `id` is an opaque string so non-UUID keys fit; read it only through
/// [`current_user`] / [`require_authenticated`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser {
    /// Stable, opaque user key; custom user tables may use any string.
    pub id: String,
    /// The identifier the user logged in with (the shipped default: email).
    pub login: String,
    pub display_name: String,
    /// Optional tenant carried from the user; the gate injects [`Tenant`] only
    /// when present.
    ///
    /// [`Tenant`]: crate::Tenant
    pub tenant_id: Option<Uuid>,
    /// Whether the user may enter the panel. Denied users answer 403,
    /// indistinguishable from bad credentials at login (ADR-0013).
    pub can_access_panel: bool,
}

/// The boxed future every [`Authenticator`] method returns.
pub type AuthFuture<'a, T> = Pin<Box<dyn Future<Output = topcoat::Result<T>> + Send + 'a>>;

/// The one authentication seam (ADR-0013).
///
/// The default [`PasswordAuth`] implements it against the shipped
/// [`AdminUser`] model; an app with an existing user table implements it and
/// passes the value to [`Panel::auth`](crate::Panel::auth) via
/// [`Auth::custom`]. Sessions are the framework's, so a custom implementation
/// only maps credentials to a [`CurrentUser`] and back.
///
/// Every *credential* failure must return `Ok(None)`, never a distinguishable
/// error: the login response is one generic message for all of them. An
/// infrastructure failure is not a credential verdict, so an implementation
/// that cannot reach its store may return the driver's error instead — the
/// login handler maps it to the opaque outage page, which is what
/// keeps a database outage from rendering as a rejected password. An
/// implementation's own error keeps its own mapping.
pub trait Authenticator: Send + Sync + 'static {
    /// Verify `login`/`password`, returning the user on success.
    ///
    /// Implementations must run comparable work for unknown accounts (the
    /// shipped [`PasswordAuth`] verifies a dummy hash) so timing does not leak
    /// account existence.
    fn verify<'a>(
        &'a self,
        cx: &'a Cx,
        login: &'a str,
        password: &'a str,
    ) -> AuthFuture<'a, Option<CurrentUser>>;

    /// Resolve the live session user by [`CurrentUser::id`].
    ///
    /// Loading the row each request is what makes deactivation and revocation
    /// take effect immediately; return `None` when the user does not
    /// authenticate.
    fn find_by_id<'a>(&'a self, cx: &'a Cx, id: &'a str) -> AuthFuture<'a, Option<CurrentUser>>;
}

/// The opaque error an infrastructure failure on an auth or session path
/// carries.
///
/// Its `Display` is [`UNAVAILABLE_ERROR`], so nothing driver-shaped can travel
/// inside it, and its concrete type is what lets the login handler recognise
/// the one case it answers with the outage page rather than a 500.
#[derive(Debug)]
struct Unavailable;

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(UNAVAILABLE_ERROR)
    }
}

impl std::error::Error for Unavailable {}

/// Map a failed auth or session operation to the error the response carries
/// The counterpart of [`crate::db::hook_failure`] for this module.
///
/// An error that is the driver's is an infrastructure failure: it becomes
/// [`Unavailable`], and the driver's own text goes to the log under this
/// event, never to the page. Anything else is app-authored (a custom
/// [`Authenticator`]'s own error) and keeps its mapping, so the seam does not
/// swallow it. Every auth and session path maps through here, which is what
/// keeps a database outage from reading as a rejected password — and what
/// makes the two distinguishable in the logs: a rejection is `Ok(None)`, the
/// generic 403 and no error line, while an outage logs the driver's text.
fn infrastructure_failure(error: impl Into<topcoat::Error>) -> topcoat::Error {
    let error = error.into();
    if error.is::<toasty::Error>() {
        tracing::error!(error = %error, "auth infrastructure failure");
        Unavailable.into()
    } else {
        error
    }
}

/// The shipped default authenticator: Argon2id verification against
/// [`AdminUser`].
#[derive(Debug, Default, Clone, Copy)]
pub struct PasswordAuth;

impl PasswordAuth {
    /// Creates the shipped authenticator.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Authenticator for PasswordAuth {
    fn verify<'a>(
        &'a self,
        cx: &'a Cx,
        login: &'a str,
        password: &'a str,
    ) -> AuthFuture<'a, Option<CurrentUser>> {
        Box::pin(async move {
            let mut db = crate::db::db(cx);
            let found = AdminUser::filter(AdminUser::fields().email().eq(login.trim().to_string()))
                .first()
                .exec(&mut db)
                .await
                .map_err(infrastructure_failure)?;
            let Some(user) = found else {
                // Unknown account: pay a verification anyway (ADR-0013).
                let _ = verify_password(password, DUMMY_PASSWORD_HASH);
                return Ok(None);
            };
            if !verify_password(password, &user.password_hash) {
                return Ok(None);
            }
            Ok(Some(current_user_from(&user)))
        })
    }

    fn find_by_id<'a>(&'a self, cx: &'a Cx, id: &'a str) -> AuthFuture<'a, Option<CurrentUser>> {
        Box::pin(async move {
            let Ok(id) = Uuid::parse_str(id) else {
                return Ok(None);
            };
            let mut db = crate::db::db(cx);
            let user = AdminUser::filter(AdminUser::fields().id().eq(id))
                .first()
                .exec(&mut db)
                .await
                .map_err(infrastructure_failure)?;
            // A deactivated account stops resolving: its live sessions are
            // purged and the next request redirects to login (spec #127 US11).
            Ok(user
                .filter(|user| user.active)
                .map(|user| current_user_from(&user)))
        })
    }
}

/// Hash a password with Argon2id into a PHC string (the shipped storage
/// format). Seeds and record fns call this; it never stores the plaintext.
pub fn hash_password(password: &str) -> topcoat::Result<String> {
    use argon2::password_hash::PasswordHasher;

    argon2::Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(topcoat::Error::from)
}

/// Verify a password against a PHC hash; `false` on any malformed input.
fn verify_password(password: &str, phc: &str) -> bool {
    use argon2::password_hash::{PasswordVerifier, phc::PasswordHash};

    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Map the shipped model onto the erased identity.
fn current_user_from(user: &AdminUser) -> CurrentUser {
    CurrentUser {
        id: user.id.to_string(),
        login: user.email.clone(),
        display_name: user.display_name.clone(),
        tenant_id: user.tenant_id,
        can_access_panel: user.active,
    }
}

/// The panel's authentication configuration (ADR-0013).
///
/// The default is [`Auth::password`]; [`Auth::custom`] swaps in an app-owned
/// [`Authenticator`]; [`Auth::disabled`] is the explicit fail-open opt-out.
pub enum Auth {
    /// The shipped Argon2id + [`AdminUser`] authenticator.
    Password(PasswordAuth),
    /// An app-owned authenticator over its own user table.
    Custom(Box<dyn Authenticator>),
    /// Explicit opt-out: no gate, no login routes, sessions unused.
    Disabled,
}

impl Auth {
    /// The shipped default.
    #[must_use]
    pub fn password() -> Self {
        Self::Password(PasswordAuth::new())
    }

    /// An app-owned authenticator, erased to the trait object the panel keeps.
    #[must_use]
    pub fn custom(authenticator: impl Authenticator) -> Self {
        Self::Custom(Box::new(authenticator))
    }

    /// Explicit fail-open opt-out for public demos (ADR-0013).
    #[must_use]
    pub fn disabled() -> Self {
        Self::Disabled
    }

    /// The authenticator, or `None` when auth is explicitly disabled.
    #[must_use]
    pub fn authenticator(&self) -> Option<&dyn Authenticator> {
        match self {
            Self::Password(password) => Some(password),
            Self::Custom(authenticator) => Some(&**authenticator),
            Self::Disabled => None,
        }
    }

    /// Whether the explicit opt-out is set.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self::password()
    }
}

/// Whether the request's panel gates (auth is installed and not disabled).
pub fn enforced(cx: &Cx) -> bool {
    try_app_context::<Auth>(cx).is_some_and(|auth| !auth.is_disabled())
}

/// The resolved identity for this request, if any.
///
/// This is the only read path for pages and app code; the gate places the
/// value in request `Cx` (ADR-0013).
pub fn current_user(cx: &Cx) -> Option<CurrentUser> {
    try_request_context::<CurrentUser>(cx).cloned()
}

/// Require an authenticated, panel-permitted user.
///
/// Answers per request kind (ADR-0013): pages redirect to the login route
/// with a same-origin-relative `next`, runtime endpoints, non-GET requests,
/// and page re-runs (marked POSTs the runtime layer rewrites into GETs)
/// answer 401, and an authenticated user without panel access answers 403.
pub fn require_authenticated(cx: &Cx) -> topcoat::Result<CurrentUser> {
    if let Some(user) = current_user(cx) {
        if user.can_access_panel {
            return Ok(user);
        }
        return Err(forbidden().into());
    }
    Err(unauthenticated_error(cx))
}

/// The error an unauthenticated request answers with, by request kind.
fn unauthenticated_error(cx: &Cx) -> topcoat::Error {
    let path = uri(cx).path();
    let page_method = matches!(*method(cx), http::Method::GET | http::Method::HEAD);
    // A page re-run reaches the gate as a rewritten GET: Topcoat's runtime
    // layer rewrites the browser's marked POST into a GET for the page's own
    // URL. The original request tells it apart from a plain page load, so a
    // logged-out re-run answers 401 like any other non-page request instead
    // of redirecting into the login page.
    let rerun = !matches!(*original_method(cx), http::Method::GET | http::Method::HEAD)
        && original_headers(cx).get(&topcoat::runtime::RUNTIME_HEADER) == Some(&RERUN_MARKER);
    if path.starts_with(RUNTIME_PREFIX) || !page_method || rerun {
        unauthorized().into()
    } else {
        redirect(login_url_with_next(cx)).into()
    }
}

/// Where the panel's login page lives: `{prefix}/login`.
fn login_url(cx: &Cx) -> String {
    format!("{}/login", panel_prefix(cx))
}

/// Where the shell's logout control posts: `{prefix}/logout`.
pub(crate) fn logout_url(cx: &Cx) -> String {
    format!("{}/logout", panel_prefix(cx))
}

/// The login URL with a validated `next` back to the requested page.
fn login_url_with_next(cx: &Cx) -> String {
    let mut url = login_url(cx);
    let request = uri(cx);
    let requested = match request.query() {
        Some(query) => format!("{}?{query}", request.path()),
        None => request.path().to_string(),
    };
    if let Some(next) = safe_next(&requested) {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        serializer.append_pair(NEXT_FIELD, next);
        url.push('?');
        url.push_str(&serializer.finish());
    }
    url
}

/// The panel root: where a completed login lands without a `next`.
fn panel_root(cx: &Cx) -> String {
    panel_prefix(cx)
}

/// The mount prefix of the panel that built this router.
fn panel_prefix(cx: &Cx) -> String {
    try_app_context::<PanelPrefix>(cx)
        .map(|prefix| prefix.0.clone())
        .unwrap_or_else(|| "/admin".to_string())
}

/// Accept only same-origin relative paths as a post-login destination
/// (ADR-0013): absolute URLs, scheme-relative `//host` targets, backslash
/// tricks, and control characters are rejected.
pub(crate) fn safe_next(next: &str) -> Option<&str> {
    let next = next.trim();
    if !next.starts_with('/') || next.starts_with("//") {
        return None;
    }
    if next.contains('\\') || next.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(next)
}

/// The validated `?next=` the login page embeds as a hidden field.
fn next_from_query(cx: &Cx) -> Option<String> {
    let query = uri(cx).query()?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == NEXT_FIELD)
        .map(|(_, value)| value.into_owned())
        .filter(|value| safe_next(value).is_some())
}

/// Hex-encode a token hash into its storage key.
fn token_key(hash: &TokenHash) -> String {
    use std::fmt::Write as _;

    let mut key = String::with_capacity(64);
    for byte in hash.iter() {
        write!(key, "{byte:02x}").expect("writing to a String cannot fail");
    }
    key
}

/// Delete the session row a token hash names, if any.
async fn delete_session(cx: &Cx, hash: &TokenHash) -> topcoat::Result<()> {
    delete_session_row(cx, &token_key(hash)).await
}

/// Delete one stored session row by its hex token-hash key.
async fn delete_session_row(cx: &Cx, key: &str) -> topcoat::Result<()> {
    let mut db = crate::db::db(cx);
    AuthSession::filter(AuthSession::fields().token_hash().eq(key.to_string()))
        .delete()
        .exec(&mut db)
        .await
        .map_err(infrastructure_failure)?;
    Ok(())
}

/// Revoke every live session of `user_id` (ADR-0013).
///
/// Call this whenever a credential changes out from under live sessions —
/// password reset/change and deactivation alike. Nothing in-core calls it
/// (there is no password-change flow in the framework); sessions otherwise
/// stay valid for their full fixed lifetime, so a reset that skips this
/// leaves a stolen session usable. A password-reset flow must call it,
/// and custom `Authenticator` apps own the same obligation.
pub async fn revoke_sessions_for_user(cx: &Cx, user_id: &str) -> topcoat::Result<()> {
    let mut db = crate::db::db(cx);
    AuthSession::filter(AuthSession::fields().user_id().eq(user_id.to_string()))
        .delete()
        .exec(&mut db)
        .await
        .map_err(infrastructure_failure)?;
    Ok(())
}

/// Drop the expired session rows of `user_id`.
///
/// [`resolve`] purges a session row when its token is looked up expired, so
/// without this a row whose token is never presented again would stay in the
/// table; a sweep that does not wait for the owner is tracked in GH #302.
/// Login is the bounded sweep: the user is present, the table is already open,
/// and only their rows are touched. Revocation ([`revoke_sessions_for_user`]) is the
/// unbounded counterpart that drops the live rows too.
async fn purge_expired_sessions_for_user(cx: &Cx, user_id: &str) -> topcoat::Result<()> {
    let now = Timestamp::now();
    let mut db = crate::db::db(cx);
    AuthSession::filter(
        AuthSession::fields()
            .user_id()
            .eq(user_id.to_string())
            .and(AuthSession::fields().expires_at().le(now)),
    )
    .delete()
    .exec(&mut db)
    .await
    .map_err(infrastructure_failure)?;
    Ok(())
}

/// Resolve the request's session into a user, lazily and without touching the
/// database when no session cookie is present.
pub(crate) async fn resolve(
    cx: &Cx,
    authenticator: &dyn Authenticator,
) -> topcoat::Result<Option<CurrentUser>> {
    let Some(hash) = session::token_hash(cx).await? else {
        return Ok(None);
    };
    let key = token_key(&hash);
    let mut db = crate::db::db(cx);
    let row = AuthSession::filter(AuthSession::fields().token_hash().eq(key))
        .first()
        .exec(&mut db)
        .await
        .map_err(infrastructure_failure)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.expires_at <= Timestamp::now() {
        // Expired sessions do not resolve; purge the row on the way out.
        delete_session_row(cx, &row.token_hash).await?;
        return Ok(None);
    }
    match authenticator
        .find_by_id(cx, &row.user_id)
        .await
        .map_err(infrastructure_failure)?
    {
        Some(user) => Ok(Some(user)),
        None => {
            // The session names a user who no longer authenticates (deleted
            // or deactivated): purge it so removal is real (US11).
            delete_session_row(cx, &row.token_hash).await?;
            Ok(None)
        }
    }
}

/// The layer that gates the panel and runtime prefixes (ADR-0013): resolves
/// the session into request `Cx` when present and answers fail-closed when a
/// route inside its prefix has no permitted user.
pub(crate) struct AuthGate {
    path: PathBuf,
}

impl AuthGate {
    /// Guards requests under `path`.
    pub(crate) fn new(path: &str) -> Self {
        Self {
            path: route_path(path),
        }
    }
}

impl Layer for AuthGate {
    fn path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            // The login page must answer while logged out.
            if uri(cx).path() == login_url(cx) {
                return next.run(cx, body).await;
            }
            let auth = app_context::<Auth>(cx);
            let Some(authenticator) = auth.authenticator() else {
                return next.run(cx, body).await;
            };
            // The logout route must answer for any resolved user, even one
            // whose panel access was revoked after login — clearing
            // the session row + cookie must not require panel permission, or
            // the session lingers to expiry. The bypass is POST-only at the
            // exact logout path: the route table registers nothing else
            // there, and a non-POST method must not smuggle a resolved
            // identity to any handler an app might mount at the same path.
            let logout_route =
                uri(cx).path() == logout_url(cx) && matches!(*method(cx), http::Method::POST);
            match resolve(cx, authenticator).await? {
                Some(user) if user.can_access_panel || logout_route => {
                    // The logged-in user's optional tenant becomes the request
                    // tenant; auth never requires one (ADR-0013).
                    let tenant_id = user.tenant_id;
                    let mut child = cx.with(user);
                    if let Some(tenant_id) = tenant_id {
                        child = child.with(crate::tenancy::Tenant(tenant_id));
                    }
                    next.run(&child, body).await
                }
                // Authenticated but not permitted: 403, indistinguishable
                // from bad credentials at login (ADR-0013). The logout route
                // is answered above.
                Some(_) => Err(forbidden().into()),
                // Pages redirect to the login route with a validated `next`;
                // runtime endpoints, non-GET requests, and page re-runs
                // answer 401.
                None => Err(unauthenticated_error(cx)),
            }
        })
    }
}

/// Install session support and the gate layers on a panel router.
pub(crate) fn install(
    builder: topcoat::router::RouterBuilder,
    prefix: &str,
) -> topcoat::router::RouterBuilder {
    builder
        .sessions(SessionConfig::builder().lifetime(SESSION_LIFETIME).build())
        .layer(AuthGate::new(prefix))
        .layer(AuthGate::new(RUNTIME_PREFIX))
}

/// What a failed login attempt renders: the generic credential rejection, or
/// the sign-in outage.
///
/// Two variants and no more, each with its own copy and status, so a failed
/// attempt is always a deliberate answer: a driver failure is never rendered
/// as a credential verdict, and a credential verdict never borrows the outage
/// copy. Neither variant carries driver text.
#[derive(Debug, Clone, Copy)]
enum LoginError {
    /// Wrong password, unknown account, empty fields, or valid credentials
    /// without panel access: one 403 with one message (ADR-0013).
    Credentials,
    /// The database behind sign-in could not answer: a 503 that says so,
    /// because the user must not read an outage as a rejected password.
    Unavailable,
}

impl LoginError {
    /// The message the login page's alert carries.
    fn message(self) -> &'static str {
        match self {
            Self::Credentials => GENERIC_ERROR,
            Self::Unavailable => UNAVAILABLE_ERROR,
        }
    }

    /// The status a failed attempt answers with.
    fn status(self) -> http::StatusCode {
        match self {
            Self::Credentials => http::StatusCode::FORBIDDEN,
            Self::Unavailable => http::StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

/// The status and route a login attempt renders: a [`LoginError`] page or a
/// redirect back to `next`.
///
/// The login page is a settled view, so [`ViewExt::single`] resolves it into
/// an owned handle before the response is built — no borrowed view escapes.
async fn login_response(
    cx: &Cx,
    error: Option<LoginError>,
    next: String,
) -> topcoat::Result<topcoat::router::response::Response> {
    let page = render_login_page(cx, error, next).await?;
    page.single().await?.into_response(cx)
}

/// `GET {prefix}/login` — the standalone login page.
pub(crate) fn login_page(cx: &Cx, _body: Body) -> RouteFuture<'_> {
    let next = next_from_query(cx).unwrap_or_default();
    Box::pin(login_response(cx, None, next))
}

/// `POST {prefix}/login` — verify, rotate the session, redirect to `next`.
pub(crate) fn login_post(cx: &Cx, body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        // Login/logout carry no file parts: the values half is all they read.
        let values = crate::panel::parse_form_body(cx, body).await?.values;
        crate::csrf::verify(cx, &values)?;
        // Keep a validated destination across a failed attempt so the retry
        // form still returns where the visitor was headed (US6).
        let next = values
            .get(NEXT_FIELD)
            .and_then(|value| safe_next(value))
            .map(str::to_string)
            .or_else(|| next_from_query(cx))
            .unwrap_or_default();
        let email = values.get(LOGIN_FIELD).map(|value| value.trim());
        let password = values.get(PASSWORD_FIELD).map(String::as_str);
        let auth = app_context::<Auth>(cx);
        let verified = match (auth.authenticator(), email, password) {
            (Some(authenticator), Some(email), Some(password))
                if !email.is_empty() && !password.is_empty() =>
            {
                match authenticator.verify(cx, email, password).await {
                    Ok(user) => user,
                    // A driver failure is not a credential verdict: the page
                    // says sign-in is unavailable instead of rendering a
                    // rejection. The seam logs the driver's text and
                    // hands back an app-authored error untouched.
                    Err(error) => {
                        let error = infrastructure_failure(error);
                        if error.is::<Unavailable>() {
                            return login_response(cx, Some(LoginError::Unavailable), next).await;
                        }
                        return Err(error);
                    }
                }
            }
            _ => None,
        };
        // One path for every failure: wrong password, unknown account, empty
        // fields, or valid credentials without panel access (ADR-0013).
        let Some(user) = verified.filter(|user| user.can_access_panel) else {
            return login_response(cx, Some(LoginError::Credentials), next).await;
        };
        // Rotate on login: a token this request presented cannot be replayed.
        if let Some(hash) = session::token_hash(cx).await? {
            delete_session(cx, &hash).await?;
        }
        // Bounded housekeeping: the expired rows of the user signing
        // in go with the rotation. A failure is logged rather than fatal — a
        // credential that verified must not become a 503 because cleanup could
        // not run.
        if let Err(error) = purge_expired_sessions_for_user(cx, &user.id).await {
            tracing::error!(error = %error, "expired-session purge failed");
        }
        let session = session::start(cx).await?;
        let mut db = crate::db::db(cx);
        let recorded = toasty::create!(AuthSession {
            token_hash: token_key(&session.token_hash),
            user_id: user.id.clone(),
            expires_at: Timestamp::try_from(session.expires_at).map_err(topcoat::Error::from)?,
            created_at: Timestamp::now(),
        })
        .exec(&mut db)
        .await;
        if let Err(error) = recorded {
            // The credentials were right; the session row could not be
            // recorded. Same outage page as a failed verification — never the
            // driver's text.
            let error = infrastructure_failure(error);
            if error.is::<Unavailable>() {
                return login_response(cx, Some(LoginError::Unavailable), next).await;
            }
            return Err(error);
        }
        let target = if next.is_empty() {
            panel_root(cx)
        } else {
            next
        };
        // Success stays on the `Ok` path so `Set-Cookie` flushes
        // (upstream topcoat#126).
        topcoat::router::error::see_other(target).into_response(cx)
    })
}

/// `POST {prefix}/logout` — delete the session row and clear the cookie.
pub(crate) fn logout_post(cx: &Cx, body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        // Defense in depth: the route only exists on gated panels, but it
        // re-checks so a missing layer cannot leave logout ungated. Any
        // resolved identity may log out — the gate answers this route for a
        // `can_access_panel=false` user too, so demanding panel
        // access here would strand their session row + cookie to expiry.
        if current_user(cx).is_none() {
            return Err(unauthenticated_error(cx));
        }
        // Login/logout carry no file parts: the values half is all they read.
        let values = crate::panel::parse_form_body(cx, body).await?.values;
        crate::csrf::verify(cx, &values)?;
        if let Some(hash) = session::stop(cx).await? {
            delete_session(cx, &hash).await?;
        }
        let target = login_url(cx);
        topcoat::router::error::see_other(target).into_response(cx)
    })
}

/// The standalone login document: brand and dark mode honored, CSRF hidden
/// field, one error slot carrying a [`LoginError`]'s deliberate copy, no
/// sidebar (ADR-0013).
async fn render_login_page<'a>(
    cx: &'a Cx,
    error: Option<LoginError>,
    next: String,
) -> topcoat::Result<BoxView<'a>> {
    let csrf = crate::csrf::ensure_token(cx);
    let action = login_url(cx);
    let brand = Panel::render_brand(cx).await?;
    let hint = try_app_context::<LoginHint>(cx).map(|hint| hint.0.clone());
    let body = topcoat::view::view! {
        cx =>
        <div class="flex min-h-svh items-center justify-center bg-muted p-6">
            <div
                class="flex w-full max-w-sm flex-col gap-6 rounded-xl border border-border bg-card p-6 text-card-foreground shadow-sm"
            >
                if let Some(error) = error {
                    (error.status())
                }
                <div class="flex flex-col items-center gap-2">
                    (brand)
                    <h1 class="text-lg font-semibold text-foreground">"Sign in"</h1>
                </div>
                <form method="post" action=(action) class="flex flex-col gap-4">
                    (crate::csrf::field(cx, &csrf))
                    <input type="hidden" name=(NEXT_FIELD) value=(next)>
                    if let Some(error) = error {
                        tablo_ui::alert(
                            variant: tablo_ui::AlertVariant::Destructive,
                            attrs: topcoat::view::attributes! { role="alert" },
                            tablo_ui::alert_title((error.message()))
                        )
                    }
                    tablo_ui::field(
                        tablo_ui::field_label(
                            attrs: topcoat::view::attributes! { for="email" },
                            "Email or username"
                        )
                        tablo_ui::input(
                            attrs: topcoat::view::attributes! {
                                id="email"
                                name=(LOGIN_FIELD)
                                type="text"
                                required=""
                                autocomplete="username"
                                autofocus=""
                            }
                        )
                    )
                    tablo_ui::field(
                        tablo_ui::field_label(
                            attrs: topcoat::view::attributes! { for="password" },
                            "Password"
                        )
                        tablo_ui::input(
                            attrs: topcoat::view::attributes! {
                                id="password"
                                name=(PASSWORD_FIELD)
                                type="password"
                                required=""
                                autocomplete="current-password"
                            }
                        )
                    )
                    tablo_ui::button(
                        variant: tablo_ui::ButtonVariant::Primary,
                        attrs: topcoat::view::attributes! { type="submit" class="w-full" },
                        "Sign in"
                    )
                </form>
                if let Some(hint) = hint {
                    <p class="text-center text-xs text-muted-foreground">(hint)</p>
                }
            </div>
        </div>
    }
    .boxed();
    Panel::render_document(cx, "Sign in".to_string(), body).await
}

/// The runtime prefix whose unauthenticated requests answer 401 instead of a
/// redirect.
pub(crate) const RUNTIME_PREFIX: &str = "/_topcoat/runtime";

/// The runtime-header value marking a page re-run POST (`true`, per Topcoat's
/// page-rerun protocol).
static RERUN_MARKER: http::HeaderValue = http::HeaderValue::from_static("true");

/// Fail loudly at startup when a required shipped model is missing from the
/// app's `Db` (ADR-0013): the table is never pushed, and the first login
/// would otherwise be a confusing runtime error.
pub(crate) fn assert_models_registered(db: &Db, auth: &Auth) {
    if auth.is_disabled() {
        return;
    }
    let registered = |name: &str| {
        db.schema()
            .app
            .models()
            .any(|model| model.name().upper_camel_case() == name)
    };
    let missing: Vec<&str> = [
        (!registered("AuthSession")).then_some("AuthSession"),
        (matches!(auth, Auth::Password(_)) && !registered("AdminUser")).then_some("AdminUser"),
    ]
    .into_iter()
    .flatten()
    .collect();
    assert!(
        missing.is_empty(),
        "tablo auth is on by default but its shipped models are not registered on the Db \
         (missing {}). Register them with `toasty::models!(…, tablo_core::auth::AdminUser, \
         tablo_core::auth::AuthSession)`, or opt out with `.auth(Auth::disabled())`.",
        missing.join(", "),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_next_only_accepts_same_origin_relative_paths() {
        assert_eq!(safe_next("/admin/users"), Some("/admin/users"));
        assert_eq!(
            safe_next("/admin/posts?status=draft"),
            Some("/admin/posts?status=draft")
        );
        assert_eq!(safe_next("  /admin  "), Some("/admin"));
        for target in [
            "",
            "admin",
            "//evil.example/login",
            "https://evil.example/steal",
            "/\\evil.example",
            "/admin\r\nLocation: https://evil.example",
        ] {
            assert_eq!(safe_next(target), None, "must reject {target:?}");
        }
    }

    #[test]
    fn password_hashes_verify_round_trip() {
        let phc = hash_password("correct horse battery staple").expect("hash");
        assert!(phc.starts_with("$argon2id$"), "{phc}");
        assert!(verify_password("correct horse battery staple", &phc));
        assert!(!verify_password("wrong", &phc));
        assert!(!verify_password("anything", "not-a-phc"));
    }

    #[test]
    fn token_keys_are_hex_encoded_sha256() {
        let token = topcoat::session::Token::random();
        let key = token_key(&token.hash());
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// A `Db` that declares the shipped auth models but never pushed their
    /// schema: the tables are missing, so the first statement fails at the
    /// driver — the setup GH #229's write-path tests use.
    async fn schema_less_db() -> Db {
        Db::builder()
            .models(toasty::models!(AdminUser, AuthSession))
            .connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite")
    }

    /// GH #230: an auth or session operation that fails at the driver is an
    /// infrastructure failure, so it carries the opaque sign-in copy — never
    /// the driver's text, and never the login page's credential rejection.
    #[test]
    fn infrastructure_failure_maps_driver_errors_to_the_opaque_sign_in_copy() {
        let err = super::infrastructure_failure(toasty::Error::from_args(format_args!(
            "secret driver gunk: no such table"
        )));
        let rendered = err.to_string();
        assert!(
            rendered.contains(UNAVAILABLE_ERROR),
            "the opaque message must survive, got {rendered}"
        );
        assert!(
            !rendered.contains("gunk"),
            "driver text must not leak, got {rendered}"
        );
        assert!(
            !rendered.contains(GENERIC_ERROR),
            "an outage must not read as a rejected password, got {rendered}"
        );
    }

    /// GH #230 must not swallow an app-authored error: a custom
    /// `Authenticator`'s own failure keeps its mapping, the way GH #229 keeps a
    /// record hook's.
    #[test]
    fn infrastructure_failure_keeps_an_app_error_intact() {
        let guard: topcoat::Error = topcoat::router::error::not_found().into();
        assert!(
            super::infrastructure_failure(guard).is::<topcoat::router::error::NotFoundError>(),
            "an app-authored error must keep its own mapping"
        );
    }

    /// A `Cx` for a login POST: the request parts the handler reads (method,
    /// content type, CSRF cookie) plus the cookie jar, over the shipped
    /// password authenticator. `token` is both the CSRF cookie and the form
    /// value the caller submits.
    fn login_cx(db: Db, token: &str) -> Cx {
        use topcoat::{context::CxTestBuilder, cookie::CookieJarCell};

        let parts = http::Request::builder()
            .method(http::Method::POST)
            .uri("/admin/login")
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(
                http::header::COOKIE,
                format!("{}={token}", crate::csrf::COOKIE_NAME),
            )
            .body(())
            .unwrap()
            .into_parts()
            .0;
        CxTestBuilder::new()
            .app_context(db)
            .app_context(Auth::password())
            .request_context(parts)
            .request_context(CookieJarCell::new())
            .build()
    }

    /// Runs the real login handler and renders the status and body a browser
    /// would be handed.
    async fn post_login(cx: &Cx, form: String) -> (http::StatusCode, String) {
        let response = login_post(cx, Body::from(form))
            .await
            .expect("a failed sign-in is answered by the login page");
        let status = response.status();
        let body = String::from_utf8_lossy(
            &http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes(),
        )
        .to_string();
        (status, body)
    }

    /// GH #230, login half: a database failure during credential verification
    /// must not echo the driver's text — the property `db.rs` pins for
    /// `unavailable`, reached through the real login handler — and must not
    /// render the login page's credential copy either, or an outage would tell
    /// the user their password was wrong.
    ///
    /// Positive-controlled like GH #229's write-path tests: the same query is
    /// asserted to carry driver text outside the handler first, so the
    /// assertions below cannot pass vacuously.
    #[tokio::test]
    async fn a_driver_login_failure_does_not_echo_driver_text() {
        // Schema never pushed: the credential query cannot run, so the failure
        // is the driver's own.
        let db = schema_less_db().await;

        // Positive control: the same query outside the handler really does
        // carry driver text.
        let mut raw = db.clone();
        let driver = AdminUser::filter(
            AdminUser::fields()
                .email()
                .eq("ada@example.com".to_string()),
        )
        .first()
        .exec(&mut raw)
        .await
        .expect_err("the table is missing")
        .to_string();
        drop(raw);
        assert!(
            driver.contains("no such table"),
            "the control must be a driver failure, got {driver:?}"
        );

        let token = Uuid::new_v4().to_string();
        let cx = login_cx(db, &token);
        let (status, rendered) = post_login(
            &cx,
            format!("email=ada@example.com&password=opensesame&csrf_token={token}"),
        )
        .await;

        assert_eq!(
            status,
            http::StatusCode::SERVICE_UNAVAILABLE,
            "an outage is not a credential rejection"
        );
        assert!(
            rendered.contains(UNAVAILABLE_ERROR),
            "the opaque outage copy must reach the page, got {rendered:?}"
        );
        assert!(
            !rendered.contains(&driver) && !rendered.contains("no such table"),
            "driver text must not reach the response: the driver said {driver:?}, the response said {rendered:?}"
        );
        assert!(
            !rendered.contains(GENERIC_ERROR),
            "an outage must not read as a rejected password, got {rendered:?}"
        );
    }

    /// GH #230, the other half: a genuine credential rejection keeps the login
    /// page's generic 403 — the same setup as the outage test with a working
    /// store and a wrong password, so the two answers cannot be confused in
    /// either direction.
    #[tokio::test]
    async fn a_rejected_password_still_renders_the_generic_error() {
        let mut db = schema_less_db().await;
        db.push_schema().await.expect("push schema");
        toasty::create!(AdminUser {
            email: "ada@example.com".to_string(),
            password_hash: hash_password("opensesame").expect("hash"),
            display_name: "Ada".to_string(),
            active: true,
            tenant_id: None,
            created_at: Timestamp::now(),
        })
        .exec(&mut db)
        .await
        .expect("seed admin");

        let token = Uuid::new_v4().to_string();
        let cx = login_cx(db, &token);
        let (status, rendered) = post_login(
            &cx,
            format!("email=ada@example.com&password=wrong&csrf_token={token}"),
        )
        .await;

        assert_eq!(
            status,
            http::StatusCode::FORBIDDEN,
            "a rejection is not an outage"
        );
        assert!(
            rendered.contains(GENERIC_ERROR),
            "the credential copy must survive, got {rendered:?}"
        );
        assert!(
            !rendered.contains(UNAVAILABLE_ERROR),
            "a rejected password must not read as an outage, got {rendered:?}"
        );
    }

    /// A router for a password-auth panel over `db`.
    fn auth_router(db: Db) -> topcoat::router::Router {
        Panel::new("admin")
            .app_context(db)
            .auth(Auth::password())
            .build()
            .expect("panel builds")
    }

    /// A login POST as the router sees it: urlencoded, carrying the CSRF cookie
    /// the double-submit check reads.
    fn login_request(body: String, csrf: &str) -> http::Request<Body> {
        http::Request::builder()
            .method(http::Method::POST)
            .uri("/admin/login")
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(
                http::header::COOKIE,
                format!("{}={csrf}", crate::csrf::COOKIE_NAME),
            )
            .body(Body::from(body))
            .unwrap()
    }

    /// The `Set-Cookie` header for the session token, when the response set one.
    fn session_cookie(response: &http::Response<Body>) -> Option<String> {
        response
            .headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with("__Host-session="))
            .map(str::to_string)
    }

    /// A `Db` with the shipped auth models, schema pushed, and one active admin.
    async fn db_with_admin(email: &str) -> Db {
        let mut db = Db::builder()
            .models(toasty::models!(AdminUser, AuthSession))
            .connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite");
        db.push_schema().await.expect("push schema");
        toasty::create!(AdminUser {
            email: email.to_string(),
            password_hash: hash_password("opensesame").expect("hash"),
            display_name: "Ada".to_string(),
            active: true,
            tenant_id: None,
            created_at: Timestamp::now(),
        })
        .exec(&mut db)
        .await
        .expect("seed admin");
        db
    }

    /// GH #295: the login route caps its body at a credential form's size, not
    /// the panel's 10 MiB form cap.
    #[tokio::test]
    async fn an_oversized_login_post_is_refused() {
        let db = db_with_admin("ada@example.com").await;
        let router = auth_router(db);

        let token = Uuid::new_v4().to_string();
        let oversized = format!(
            "email={}&password=opensesame&csrf_token={token}",
            "a".repeat(MAX_LOGIN_BYTES)
        );
        let resp = router.handle(login_request(oversized, &token)).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "a login body over the credential cap must be refused, got {}",
            resp.status()
        );
        assert!(
            session_cookie(&resp).is_none(),
            "a refused login must not start a session"
        );

        // A normal credential POST still signs in.
        let token = Uuid::new_v4().to_string();
        let resp = router
            .handle(login_request(
                format!("email=ada@example.com&password=opensesame&csrf_token={token}"),
                &token,
            ))
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::SEE_OTHER,
            "a normal credential POST must sign in, got {}",
            resp.status()
        );
        assert!(
            session_cookie(&resp).is_some(),
            "a successful login starts a session"
        );
    }

    /// GH #295: login drops the signing-in user's expired session rows. A row
    /// whose token is never presented again would otherwise stay in the table
    /// forever, because [`resolve`] only purges a row it looks up.
    #[tokio::test]
    async fn login_purges_the_users_expired_sessions() {
        let mut db = db_with_admin("ada@example.com").await;
        let other = toasty::create!(AdminUser {
            email: "grace@example.com".to_string(),
            password_hash: hash_password("opensesame").expect("hash"),
            display_name: "Grace".to_string(),
            active: true,
            tenant_id: None,
            created_at: Timestamp::now(),
        })
        .exec(&mut db)
        .await
        .expect("seed the other admin");
        let ada = AdminUser::filter(
            AdminUser::fields()
                .email()
                .eq("ada@example.com".to_string()),
        )
        .first()
        .exec(&mut db)
        .await
        .expect("look up ada")
        .expect("ada exists");
        let expired = "2000-01-01T00:00:00Z"
            .parse::<Timestamp>()
            .expect("a past timestamp");
        let live = "2100-01-01T00:00:00Z"
            .parse::<Timestamp>()
            .expect("a future timestamp");
        for (token_hash, user_id, expires_at) in [
            ("expired-mine", ada.id.to_string(), expired),
            ("live-mine", ada.id.to_string(), live),
            ("expired-other", other.id.to_string(), expired),
        ] {
            toasty::create!(AuthSession {
                token_hash: token_hash.to_string(),
                user_id,
                expires_at,
                created_at: Timestamp::now(),
            })
            .exec(&mut db)
            .await
            .expect("seed a session row");
        }

        let router = auth_router(db.clone());
        let token = Uuid::new_v4().to_string();
        let resp = router
            .handle(login_request(
                format!("email=ada@example.com&password=opensesame&csrf_token={token}"),
                &token,
            ))
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::SEE_OTHER,
            "the login must succeed"
        );

        let mut check = db.clone();
        assert!(
            AuthSession::filter(
                AuthSession::fields()
                    .token_hash()
                    .eq("expired-mine".to_string())
            )
            .first()
            .exec(&mut check)
            .await
            .expect("query")
            .is_none(),
            "the signing-in user's expired session must be purged"
        );
        let mut check = db.clone();
        assert!(
            AuthSession::filter(
                AuthSession::fields()
                    .token_hash()
                    .eq("live-mine".to_string())
            )
            .first()
            .exec(&mut check)
            .await
            .expect("query")
            .is_some(),
            "a live session of the same user survives the purge"
        );
        let mut check = db.clone();
        assert!(
            AuthSession::filter(
                AuthSession::fields()
                    .token_hash()
                    .eq("expired-other".to_string())
            )
            .first()
            .exec(&mut check)
            .await
            .expect("query")
            .is_some(),
            "another user's expired session is out of the sweep's scope"
        );
    }

    /// GH #230, session half: the session-row paths map through the same seam,
    /// so a delete that fails at the driver answers the opaque copy too —
    /// driver text stays in the log there as well.
    #[tokio::test]
    async fn a_driver_session_delete_failure_does_not_echo_driver_text() {
        use topcoat::context::CxTestBuilder;

        let db = schema_less_db().await;

        // Positive control: the same delete outside the handler really does
        // carry driver text.
        let mut raw = db.clone();
        let driver = AuthSession::filter(AuthSession::fields().user_id().eq("ada".to_string()))
            .delete()
            .exec(&mut raw)
            .await
            .expect_err("the table is missing")
            .to_string();
        drop(raw);
        assert!(
            driver.contains("no such table"),
            "the control must be a driver failure, got {driver:?}"
        );

        let cx = CxTestBuilder::new().app_context(db).build();
        let error = revoke_sessions_for_user(&cx, "ada")
            .await
            .expect_err("the delete must fail");

        let rendered = error.to_string();
        assert!(
            rendered.contains(UNAVAILABLE_ERROR),
            "the opaque message must survive, got {rendered:?}"
        );
        assert!(
            !rendered.contains(&driver) && !rendered.contains("no such table"),
            "driver text must not reach the response: the driver said {driver:?}, the response said {rendered:?}"
        );
    }
}
