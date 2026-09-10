//! Authentication — credentials, server-side sessions, and the panel gate
//! (ADR-0013, spec #127).
//!
//! A [`Panel`](crate::Panel) is gated by default. The shipped [`PasswordAuth`]
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

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use jiff::Timestamp;
use toasty::Db;
use topcoat::context::{Cx, app_context, try_app_context, try_request_context};
use topcoat::router::{
    Body, Layer, LayerFuture, Next, Path, PathBuf, RouteFuture,
    error::{forbidden, redirect, unauthorized},
    request::{method, uri},
    response::IntoResponse,
};
use topcoat::session::{self, RouterBuilderSessionExt, SessionConfig, TokenHash};
use topcoat::view::{BoxView, ViewExt};
use uuid::Uuid;

use crate::panel::{LoginHint, Panel, PanelPrefix, route_path};

/// How long a session stays valid: seven days, fixed (ADR-0013).
pub const SESSION_LIFETIME: Duration = Duration::from_hours(24 * 7);

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

/// A dummy Argon2id PHC string verified against when the account does not
/// exist, so unknown emails pay the same work as known ones (ADR-0013).
/// Generated with `Argon2::default()` parameters (`m=19456,t=2,p=1`).
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$h3oXdPBVwhcgZ1OTO/PuzQ$zLrHLgIkwhqu4ZlLTfSyB8mPuL6mAtaswv/eXJ5ADO8";

/// The shipped credential model (ADR-0013): unique email, Argon2id PHC hash,
/// display name, active flag, and an optional tenant.
///
/// Register it (plus [`AuthSession`]) in the app's `Db` model list, seed one
/// row, and the default [`PasswordAuth`] works with no further wiring:
/// `toasty::models!(…, argentum_core::auth::AdminUser, argentum_core::auth::AuthSession)`.
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
/// Every failure must return `Ok(None)`, never a distinguishable error: the
/// login response is one generic message for all of them.
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
    /// take effect immediately; return `None` for a user that no longer
    /// authenticates.
    fn find_by_id<'a>(&'a self, cx: &'a Cx, id: &'a str) -> AuthFuture<'a, Option<CurrentUser>>;
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
                .map_err(topcoat::Error::from)?;
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
                .map_err(topcoat::Error::from)?;
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
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};

    let salt = SaltString::generate(&mut OsRng);
    argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(topcoat::Error::from)
}

/// Verify a password against a PHC hash; `false` on any malformed input.
fn verify_password(password: &str, phc: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};

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
/// with a same-origin-relative `next`, runtime endpoints and non-GET requests
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
    if path.starts_with(RUNTIME_PREFIX) || !page_method {
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
        .map_err(topcoat::Error::from)?;
    Ok(())
}

/// Revoke every live session of `user_id` (ADR-0013): the hook password reset
/// and deactivation call so removal is real.
pub async fn revoke_sessions_for_user(cx: &Cx, user_id: &str) -> topcoat::Result<()> {
    let mut db = crate::db::db(cx);
    AuthSession::filter(AuthSession::fields().user_id().eq(user_id.to_string()))
        .delete()
        .exec(&mut db)
        .await
        .map_err(topcoat::Error::from)?;
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
        .map_err(topcoat::Error::from)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.expires_at <= Timestamp::now() {
        // Expired sessions do not resolve; purge the row on the way out.
        delete_session_row(cx, &row.token_hash).await?;
        return Ok(None);
    }
    match authenticator.find_by_id(cx, &row.user_id).await? {
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
            match resolve(cx, authenticator).await? {
                Some(user) if user.can_access_panel => {
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
                // from bad credentials at login (ADR-0013).
                Some(_) => Err(forbidden().into()),
                // Pages redirect to the login route with a validated `next`;
                // runtime endpoints and non-GET requests answer 401.
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

/// The status and route a login attempt renders: either the generic failure
/// (403, same body for every cause) or a redirect back to `next`.
///
/// The login page is a settled view, so [`ViewExt::single`] resolves it into
/// an owned handle before the response is built — no borrowed view escapes.
async fn login_response(
    cx: &Cx,
    error: Option<String>,
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
        let values = crate::panel::parse_form_values(cx, body).await?;
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
                authenticator.verify(cx, email, password).await?
            }
            _ => None,
        };
        // One path for every failure: wrong password, unknown account, empty
        // fields, or valid credentials without panel access (ADR-0013).
        let Some(user) = verified.filter(|user| user.can_access_panel) else {
            return login_response(cx, Some(GENERIC_ERROR.to_string()), next).await;
        };
        // Rotate on login: a token this request presented cannot be replayed.
        if let Some(hash) = session::token_hash(cx).await? {
            delete_session(cx, &hash).await?;
        }
        let session = session::start(cx).await?;
        let mut db = crate::db::db(cx);
        toasty::create!(AuthSession {
            token_hash: token_key(&session.token_hash),
            user_id: user.id.clone(),
            expires_at: Timestamp::try_from(session.expires_at).map_err(topcoat::Error::from)?,
            created_at: Timestamp::now(),
        })
        .exec(&mut db)
        .await
        .map_err(topcoat::Error::from)?;
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
        // re-checks so a missing layer cannot leave logout ungated.
        crate::auth::require_authenticated(cx)?;
        let values = crate::panel::parse_form_values(cx, body).await?;
        crate::csrf::verify(cx, &values)?;
        if let Some(hash) = session::stop(cx).await? {
            delete_session(cx, &hash).await?;
        }
        let target = login_url(cx);
        topcoat::router::error::see_other(target).into_response(cx)
    })
}

/// The standalone login document: brand and dark mode honored, CSRF hidden
/// field, one generic error slot, no sidebar (ADR-0013).
async fn render_login_page<'a>(
    cx: &'a Cx,
    error: Option<String>,
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
                class="flex w-full max-w-sm flex-col gap-6 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                if error.is_some() {
                    (http::StatusCode::FORBIDDEN)
                }
                <div class="flex flex-col items-center gap-2">
                    (brand)
                    <h1 class="text-lg font-semibold text-foreground">"Sign in"</h1>
                </div>
                <form method="post" action=(action) class="flex flex-col gap-4">
                    <input type="hidden" name=(crate::csrf::FIELD_NAME) value=(csrf)>
                    <input type="hidden" name=(NEXT_FIELD) value=(next)>
                    if let Some(error) = error {
                        argentum_ui::alert(
                            variant: argentum_ui::AlertVariant::Destructive,
                            attrs: topcoat::view::attributes! { role="alert" },
                            argentum_ui::alert_title((error))
                        )
                    }
                    <div class="grid gap-2">
                        argentum_ui::label(
                            attrs: topcoat::view::attributes! { for="email" },
                            "Email or username"
                        )
                        argentum_ui::input(
                            attrs: topcoat::view::attributes! {
                                id="email"
                                name=(LOGIN_FIELD)
                                type="text"
                                required=""
                                autocomplete="username"
                                autofocus=""
                            }
                        )
                    </div>
                    <div class="grid gap-2">
                        argentum_ui::label(
                            attrs: topcoat::view::attributes! { for="password" },
                            "Password"
                        )
                        argentum_ui::input(
                            attrs: topcoat::view::attributes! {
                                id="password"
                                name=(PASSWORD_FIELD)
                                type="password"
                                required=""
                                autocomplete="current-password"
                            }
                        )
                    </div>
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Primary,
                        attrs: topcoat::view::attributes! { r#type="submit" class="w-full" },
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
        "argentum auth is on by default but its shipped models are not registered on the Db \
         (missing {}). Register them with `toasty::models!(…, argentum_core::auth::AdminUser, \
         argentum_core::auth::AuthSession)`, or opt out with `.auth(Auth::disabled())`.",
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
}
