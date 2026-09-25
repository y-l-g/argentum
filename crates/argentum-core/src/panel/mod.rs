//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`, and registers each
//! declared [`Resource`]'s list page at `{prefix}/{slug}` (Filament-style
//! routes — ADR-0008). See `CONTEXT.md`.
//!
//! Layout: [`Panel`] builder + route table live here; shell rendering in
//! `shell`, list + live shard support in `list`, form decoding and
//! create/edit in `forms`, the record detail page in `detail`,
//! delete/bulk/export in `actions`, the live-search registry + shard dispatch
//! in `search`, and response hardening headers in `headers`.

mod actions;
mod detail;
mod forms;
mod headers;
mod list;
mod search;
mod shell;
#[cfg(test)]
mod test_support;

use std::{collections::HashMap, path::PathBuf};

use toasty::{Db, schema::Model};
use topcoat::{
    Result,
    asset::{Asset, AssetConfig, RouterBuilderAssetExt},
    context::{Cx, app_context},
    cookie::RouterBuilderCookieExt,
    font::Font,
    router::{
        Body, PageFn, Path, RouteFn, RouteFuture, Router, RouterBuilderDirectoryExt,
        RouterBuilderDiscoverExt, error::redirect,
    },
    runtime::RouterBuilderRuntimeExt,
};

#[cfg(feature = "auth")]
pub(crate) use self::forms::parse_form_body;
#[cfg(test)]
pub(crate) use self::search::TABLE_SEARCH_PATH;
pub(crate) use self::search::table_search;
pub use self::shell::{Brand, DarkMode};
use self::{
    actions::{resource_bulk_delete, resource_delete, resource_export, resource_options},
    detail::resource_view,
    forms::{
        MAX_FORM_BYTES, resource_create, resource_create_post, resource_edit, resource_edit_post,
    },
    list::{declared_chrome, resource_list},
    search::{SearchFn, SearchRegistry, search_handler_for},
    shell::ShellAssets,
};
use crate::resource::{
    BULK_DELETE_ROUTE_SEGMENT, CREATE_ROUTE_SEGMENT, DELETE_ROUTE_SEGMENT, EDIT_ROUTE_SEGMENT,
    NavigationItem, RECORD_ROUTE_PARAM, Resource,
};

/// The admin application.
///
/// ```ignore
/// Panel::new("admin")
///     .app_context(db)
///     .resource::<UserResource>()
///     .build().expect("panel builds")
/// ```
pub struct Panel {
    prefix: String,
    db: Option<Db>,
    assets: Option<AssetConfig>,
    shell_assets: Option<ShellAssets>,
    brand: Option<Brand>,
    dark_mode: Option<bool>,
    nav_items: Vec<NavigationItem>,
    pages: Vec<PageFn>,
    routes: Vec<RouteFn>,
    root_target: Option<String>,
    slugs: Vec<String>,
    search_handlers: HashMap<String, SearchFn>,
    /// `Content-Security-Policy: frame-ancestors …` for every response
    /// (GH #176); `None` opts out. Defaults to `'self'`.
    frame_ancestors: Option<String>,
    /// Per-resource declaration checks (GH #138), monomorphized at
    /// `resource::<R>()` and run by `build` before anything is served.
    resource_checks: Vec<ResourceCheck>,
    /// Registration failures collected by the declarative builders
    /// (GH #174): `Panel::resource` cannot return `Result`, so a bad `slug()`
    /// or a duplicate is recorded here and reported by `build`.
    registration_errors: Vec<String>,
    /// Where `FileUpload` bytes go (GH #188); `None` keeps the pre-#188
    /// contract (the sanitized basename is the stored value).
    uploads: Option<crate::upload::InstalledUploader>,
    /// App-owned filesystem directories served from this panel's router
    /// (GH #188): `(route pattern, directory)`.
    served_dirs: Vec<(String, PathBuf)>,
    #[cfg(feature = "auth")]
    login_hint: Option<String>,
    #[cfg(feature = "auth")]
    auth: crate::auth::Auth,
    /// `true` once the app acknowledged the feature-off build with
    /// [`Panel::auth`]; [`Panel::build`] refuses the panel otherwise.
    #[cfg(not(feature = "auth"))]
    auth_disabled: bool,
}
/// Where the panel root redirects (the first declared resource's list).
/// Lives on the `app_context` because page handlers are plain `fn` pointers
/// and cannot capture.
#[derive(Debug, Clone)]
struct RootRedirect(String);
/// The mount prefix of the [`Panel`] that built this Router (e.g. `/admin`).
/// Installed by [`Panel::build`] so generic handlers can derive every
/// resource URL as `{prefix}/{slug}` — correct by construction even when a
/// table renders away from its own list route — instead of sniffing the
/// request path (GH #75 item 6).
#[derive(Debug, Clone)]
pub(crate) struct PanelPrefix(pub(crate) String);
/// Demo/deployment hint rendered under the login form (auth feature).
#[cfg(feature = "auth")]
#[derive(Debug, Clone)]
pub(crate) struct LoginHint(pub(crate) String);
impl Panel {
    /// Create a `Panel` mounted at `prefix` (e.g. `"admin"` → `"/admin"`).
    pub fn new(prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        let trimmed = raw.trim_matches('/').trim().to_string();
        let prefix = if trimmed.is_empty() {
            "/admin".to_string()
        } else {
            format!("/{trimmed}")
        };
        // The prefix is free-form too, and every route path is built from it
        // (GH #174): validate it once here rather than panicking at the first
        // `route_path` call.
        let registration_errors: Vec<String> = prefix
            .trim_matches('/')
            .split('/')
            .filter_map(|segment| validate_route_segment("panel prefix", segment).err())
            .collect();
        Self {
            prefix,
            db: None,
            assets: None,
            shell_assets: None,
            brand: None,
            dark_mode: None,
            nav_items: Vec::new(),
            pages: Vec::new(),
            routes: Vec::new(),
            root_target: None,
            slugs: Vec::new(),
            search_handlers: HashMap::new(),
            frame_ancestors: Some(headers::DEFAULT_FRAME_ANCESTORS.to_string()),
            registration_errors,
            resource_checks: Vec::new(),
            uploads: None,
            served_dirs: Vec::new(),
            #[cfg(feature = "auth")]
            login_hint: None,
            #[cfg(feature = "auth")]
            auth: crate::auth::Auth::default(),
            #[cfg(not(feature = "auth"))]
            auth_disabled: false,
        }
    }

    /// Returns the mount prefix, e.g. `"/admin"`.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Register the pooled `Db` on the `app_context`.
    pub fn app_context(mut self, db: Db) -> Self {
        self.db = Some(db);
        self
    }

    /// Install the [`Uploader`](crate::Uploader) every `FileUpload` stores
    /// through (GH #188).
    ///
    /// One per panel, on the app context the way `Db` is, because where bytes
    /// live is an app-level dependency: an object store, a directory on disk, a
    /// CDN. Without it a `FileUpload` keeps the pre-#188 contract — the
    /// sanitized client filename is the stored value — so an app that never
    /// installs one is unaffected.
    pub fn uploads(mut self, uploader: impl crate::Uploader) -> Self {
        self.uploads = Some(crate::upload::InstalledUploader::new(uploader));
        self
    }

    /// Serve a directory of files from this panel's router (GH #188).
    ///
    /// `path` is a route pattern ending in a catch-all (e.g.
    /// `"/uploads/{*file}"`), and `dir` the directory those URLs read from —
    /// see Topcoat's `DirectoryRoute` for the resolution rules. The path is
    /// **not** panel-relative: a served directory holds files a record points
    /// at (an upload store's output), which are not a page of the panel and
    /// must not move when the panel is mounted elsewhere. This is **public**:
    /// the auth gate covers only the panel prefix and `/_topcoat/runtime`
    /// (ADR-0013), so a served directory sits outside it and its URLs answer
    /// whoever asks, with no session — an app that needs protected files owns
    /// that route itself (ADR-0017, GH #225).
    ///
    /// Every file the directory route serves carries `nosniff`, a sandboxing
    /// `Content-Security-Policy`, and `Content-Disposition: attachment` for
    /// anything but common raster images, audio/video and plain text, so an
    /// uploaded document cannot run script on the panel's origin (GH #278). A
    /// 404 keeps Topcoat's `text/plain` error page; a 405 carries only `Allow`
    /// and an empty body. Neither carries user content.
    ///
    /// The Panel owns the [`Router`], so this is the app's only way to mount a
    /// route the framework does not own.
    pub fn serve_dir(mut self, path: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if !is_directory_pattern(&path) {
            self.registration_errors.push(format!(
                "serve_dir path '{path}': must end in a catch-all like '/uploads/{{*file}}'"
            ));
        }
        self.served_dirs.push((path, dir.into()));
        self
    }

    /// Register the asset bundle used by the Panel's shell and UI components.
    ///
    /// Loading the bundle is an application concern; applications should fail
    /// loudly at startup when their generated bundle is missing rather than
    /// silently serving an unstyled shell.
    pub fn assets(mut self, assets: impl Into<AssetConfig>) -> Self {
        self.assets = Some(assets.into());
        self
    }

    /// Register the generated stylesheet and font linked by the default shell.
    ///
    /// `stylesheet` is normally `tailwind::stylesheet!()` and `font` is
    /// normally a `fontsource_font!(.., host: Asset)` value from the app.
    pub fn shell_assets(mut self, stylesheet: Asset, font: Font) -> Self {
        self.shell_assets = Some(ShellAssets { stylesheet, font });
        self
    }

    /// Declare a `Resource` for this panel (the declarative seam, ADR-0008).
    ///
    /// Registers the resource's **list page** at `{prefix}/{slug}` (e.g.
    /// `Panel::new("admin").resource::<UserResource>()` serves `/admin/users`)
    /// and derives its [`NavigationItem`] from the same slug, so the sidebar
    /// and the router can never disagree. The panel root redirects to the
    /// first declared resource's list. Multiple calls compose. Sidebar order
    /// comes from the resource's [`Resource::navigation`] override, defaulting
    /// to declaration order (GH #102/#165).
    ///
    /// A duplicate slug (GH #102) or a slug that is not one URL segment
    /// (GH #174) is recorded here and reported by [`Panel::build`], which
    /// returns `Err` instead of panicking: two resources over one slug would
    /// shadow each other's routes, and a hostile `slug()` must not reach a
    /// route path or a response header.
    pub fn resource<R: Resource>(mut self) -> Self {
        let slug = R::slug();
        if let Err(error) = validate_route_segment("Resource::slug", &slug) {
            self.registration_errors.push(error);
            return self;
        }
        if self.slugs.iter().any(|s| s == &slug) {
            self.registration_errors.push(format!(
                "duplicate resource slug '{slug}': each Resource needs a distinct slug (see Resource::slug)"
            ));
            return self;
        }
        self.slugs.push(slug);
        self.resource_checks.push(check_resource::<R>);
        let url = format!("{}/{}", self.prefix, R::slug());
        self.pages.push(PageFn::new(
            http::Method::GET,
            route_path(&url),
            resource_list::<R>,
        ));
        // Create page — GET renders form, POST handles submission.
        let create_url = format!("{url}/{CREATE_ROUTE_SEGMENT}");
        self.pages.push(PageFn::new(
            http::Method::GET,
            route_path(&create_url),
            resource_create::<R>,
        ));
        self.pages.push(PageFn::new(
            http::Method::POST,
            route_path(&create_url),
            resource_create_post::<R>,
        ));
        // Detail page — GET renders the record read-only (GH #187). Registered
        // unconditionally, unlike the row link: `Panel::resource` runs before a
        // request exists, so `R::view(cx)` is not declarable here. The handler
        // 404s a resource that declares no view, which is the same answer as an
        // unknown id and costs one comparison.
        //
        // `RECORD_ROUTE_PARAM` shares its position with the literal `create`
        // segment above: topcoat routes through `matchit`, which prefers a
        // static segment over a parameter one, so the create page keeps being
        // reached regardless of registration order.
        let detail_url = format!("{url}/{RECORD_ROUTE_PARAM}");
        self.pages.push(PageFn::new(
            http::Method::GET,
            route_path(&detail_url),
            resource_view::<R>,
        ));
        // Edit page — GET renders hydrated form, POST handles update.
        let edit_url = format!("{url}/{RECORD_ROUTE_PARAM}/{EDIT_ROUTE_SEGMENT}");
        self.pages.push(PageFn::new(
            http::Method::GET,
            route_path(&edit_url),
            resource_edit::<R>,
        ));
        self.pages.push(PageFn::new(
            http::Method::POST,
            route_path(&edit_url),
            resource_edit_post::<R>,
        ));
        // Delete action — POST via row button (requires confirmation).
        let delete_url = format!("{url}/{RECORD_ROUTE_PARAM}/{DELETE_ROUTE_SEGMENT}");
        self.pages.push(PageFn::new(
            http::Method::POST,
            route_path(&delete_url),
            resource_delete::<R>,
        ));
        // Bulk delete — POST with `ids` form field (comma-separated).
        let bulk_delete_url = format!("{url}/{BULK_DELETE_ROUTE_SEGMENT}");
        self.pages.push(PageFn::new(
            http::Method::POST,
            route_path(&bulk_delete_url),
            resource_bulk_delete::<R>,
        ));
        // CSV export — GET over the tenant-scoped export query + Table
        // filters/sort (ADR-0012, GH #223).
        let export_url = format!("{}/export", url);
        self.routes.push(RouteFn::new(
            http::Method::GET,
            route_path(&export_url),
            resource_export::<R>,
        ));
        // Relationship option search — GET for searchable selects past the cap
        // (GH #150): `{list_url}/options?field=&q=` reusing the related
        // table's searchable columns, bounded, policy-checked.
        let options_url = format!("{}/options", url);
        self.routes.push(RouteFn::new(
            http::Method::GET,
            route_path(&options_url),
            resource_options::<R>,
        ));
        // Live-search handler (GH #104): the slug-dispatched `#[shard]` below
        // cannot be generic (inventory only discovers concrete fns), so each
        // resource monomorphizes its table loader here, keyed by list path.
        self.search_handlers
            .insert(url.clone(), search_handler_for::<R>());
        if self.root_target.is_none() {
            self.root_target = Some(url);
        }
        let nav_item = self.nav_item::<R>();
        self.nav_items.push(nav_item);
        self
    }

    /// Set branding for the shell (header + sidebar). Additive `class` stays the only Shell seam.
    pub fn brand(mut self, brand: Brand) -> Self {
        self.brand = Some(brand);
        self
    }

    /// Set the `frame-ancestors` directive the panel sends (GH #176).
    ///
    /// Defaults to `'self'`: the admin only frames itself, so a hostile page
    /// cannot clickjack it. Pass what your deployment needs — `"'self'
    /// https://intranet.example"` to allow an internal portal, or `"'none'"`
    /// to forbid framing outright.
    ///
    /// The layer only fills the gap, so an app that sets its own
    /// `Content-Security-Policy` (its own layer or route) keeps it. Use
    /// [`Self::without_frame_ancestors`] to send nothing at all and leave the
    /// decision to a proxy.
    pub fn frame_ancestors(mut self, ancestors: impl Into<String>) -> Self {
        self.frame_ancestors = Some(ancestors.into());
        self
    }

    /// Send no `frame-ancestors` directive (GH #176): the escape hatch for
    /// deployments whose proxy owns the whole CSP.
    ///
    /// Off by default in the sense that nothing is *added* — the panel's
    /// `'self'` default is what this opts out of.
    pub fn without_frame_ancestors(mut self) -> Self {
        self.frame_ancestors = None;
        self
    }

    /// Enable dark mode toggle persistence (cookie + localStorage via `theme.js`).
    pub fn dark_mode(mut self, enabled: bool) -> Self {
        self.dark_mode = Some(enabled);
        self
    }

    /// Configure authentication (ADR-0013).
    ///
    /// The default is the shipped [`Auth::password`](crate::auth::Auth::password)
    /// over [`AdminUser`](crate::auth::AdminUser); swap in an app-owned
    /// authenticator with `Panel::auth(Auth::custom(..))`, or opt a public
    /// demo out explicitly with `Panel::auth(Auth::disabled())`.
    #[cfg(feature = "auth")]
    pub fn auth(mut self, auth: crate::auth::Auth) -> Self {
        self.auth = auth;
        self
    }

    /// Acknowledge that this build has no authentication (ADR-0013): with the
    /// `auth` feature off, [`build`](Self::build) refuses a panel that has not
    /// been handed [`Auth::disabled`](crate::Auth::disabled).
    #[cfg(not(feature = "auth"))]
    pub fn auth(mut self, _auth: crate::Auth) -> Self {
        self.auth_disabled = true;
        self
    }

    /// A muted line rendered under the login form, for demo credentials or
    /// deployment hints (e.g. `"Demo: admin@example.com / password"`).
    #[cfg(feature = "auth")]
    pub fn login_hint(mut self, hint: impl Into<String>) -> Self {
        self.login_hint = Some(hint.into());
        self
    }

    /// Build the [`Router`], discovering all `#[page]` / `#[layout]` / `#[shard]`
    /// items linked into the binary, mounting the browser-runtime layer
    /// (`RouterBuilderRuntimeExt::runtime`, required by `runtime::script`),
    /// installing the `Db` and the panel navigation on the `app_context`,
    /// registering each declared resource's list page, and pointing the
    /// panel root at the first resource.
    ///
    /// # Errors
    ///
    /// Reports what the declarative builders could only record (GH #174):
    /// a missing [`Db`], a duplicate or malformed resource slug, a malformed
    /// panel prefix, or `shell_assets` declared without `assets`. Configuring
    /// a panel wrong is a boot failure, not a request-time panic, so it comes
    /// back as an error the caller can log or exit on.
    ///
    /// With the `auth` feature off nothing authenticates requests, so a panel
    /// that has not acknowledged that with
    /// [`Panel::auth(Auth::disabled())`](Self::auth) is also an error
    /// (ADR-0013).
    pub fn build(self) -> topcoat::Result<Router> {
        if !self.registration_errors.is_empty() {
            return Err(std::io::Error::other(format!(
                "Panel::build: {}",
                self.registration_errors.join("; ")
            ))
            .into());
        }
        if self.shell_assets.is_some() && self.assets.is_none() {
            return Err(std::io::Error::other(
                "Panel::build requires assets when shell_assets are configured",
            )
            .into());
        }
        let Panel {
            prefix,
            db,
            assets,
            shell_assets,
            brand,
            dark_mode,
            nav_items,
            pages,
            routes,
            root_target,
            slugs: _,
            search_handlers,
            frame_ancestors,
            registration_errors: _,
            resource_checks,
            uploads,
            served_dirs,
            #[cfg(feature = "auth")]
            login_hint,
            #[cfg(feature = "auth")]
            auth,
            #[cfg(not(feature = "auth"))]
            auth_disabled,
        } = self;
        let db = db.ok_or_else(|| {
            topcoat::Error::from(std::io::Error::other(
                "Panel::build requires a Db via app_context",
            ))
        })?;
        // Declaration checks (GH #138): a resource whose table or form could
        // never render is a configuration error, and the declaration is
        // knowable here — waiting for the first request only moves the failure
        // somewhere less useful. `table`, `form` and `can_create` are pure
        // declarations, so they must not need request-scoped context.
        if !resource_checks.is_empty() {
            let cx = validation_cx(&db);
            let failures: Vec<String> = resource_checks
                .iter()
                .filter_map(|check| check(&cx).err())
                .collect();
            if !failures.is_empty() {
                return Err(std::io::Error::other(format!(
                    "Panel::build: {}",
                    failures.join("; ")
                ))
                .into());
            }
        }
        // Auth compiled out (ADR-0013): `enforce_auth` is a no-op and no gate
        // is installed, so a panel that reaches here would serve every page and
        // mutation to anyone. The opt-out stays a line of app code.
        #[cfg(not(feature = "auth"))]
        if !auth_disabled {
            return Err(std::io::Error::other(
                "Panel::build: argentum-core is built without the `auth` feature, so nothing \
                 authenticates requests; call `.auth(Auth::disabled())` to serve the panel \
                 ungated, or enable the feature",
            )
            .into());
        }
        #[cfg(feature = "auth")]
        crate::auth::assert_models_registered(&db, &auth);
        let mut builder = Router::builder()
            .discover()
            .cookies()
            // Form bodies (urlencoded buffered, multipart streamed) share one
            // cap (GH #90): without this layer Topcoat's 2 MiB default would
            // 413 uploads the framework otherwise accepts.
            .layer(topcoat::router::BodyLimit::max(MAX_FORM_BYTES))
            .app_context(db);
        // Clickjacking hardening (GH #176): a response anyone can frame is a
        // threat on every deployment, so the panel ships the directive itself
        // and apps that need framing opt out (or supply their own policy,
        // which wins — the layer only fills the gap).
        if let Some(directive) = frame_ancestors {
            builder = builder.layer(headers::FrameAncestors::new(directive));
        }
        // Auth (ADR-0013): sessions plus the resolving gate under the panel
        // and runtime prefixes, and the login/logout routes. Disabled skips
        // all three but still installs the `Auth` value for the shell.
        #[cfg(feature = "auth")]
        {
            if !auth.is_disabled() {
                builder = crate::auth::install(builder, &prefix);
                let login_path = route_path(&format!("{prefix}/login"));
                let logout_path = route_path(&format!("{prefix}/logout"));
                // A credential POST carries no upload (GH #295): the login route
                // gets its own cap, scoped by path so it wins over the panel's
                // 10 MiB form cap.
                builder = builder.layer(
                    topcoat::router::BodyLimit::max(crate::auth::MAX_LOGIN_BYTES)
                        .at(login_path.clone()),
                );
                builder = builder
                    .route(RouteFn::new(
                        http::Method::GET,
                        login_path.clone(),
                        crate::auth::login_page,
                    ))
                    .route(RouteFn::new(
                        http::Method::POST,
                        login_path,
                        crate::auth::login_post,
                    ))
                    .route(RouteFn::new(
                        http::Method::POST,
                        logout_path,
                        crate::auth::logout_post,
                    ));
            }
        }
        if !search_handlers.is_empty() {
            builder = builder.app_context(SearchRegistry(search_handlers));
        }
        // The mount prefix travels with the Router so generic handlers derive
        // resource URLs from the declaration instead of sniffing the request
        // path (GH #75 item 6 / B4).
        builder = builder.app_context(PanelPrefix(prefix.clone()));
        if !nav_items.is_empty() {
            builder = builder.app_context(nav_items);
        }
        if let Some(assets) = assets {
            builder = builder.assets(assets);
        }
        if let Some(shell_assets) = shell_assets {
            builder = builder.app_context(shell_assets);
        }
        if let Some(brand) = brand {
            builder = builder.app_context(brand);
        }
        if let Some(enabled) = dark_mode {
            builder = builder.app_context(DarkMode(enabled));
        }
        // Where uploaded bytes go (GH #188): installed once, found by the form
        // handlers and the multipart parser through the app context.
        if let Some(uploads) = uploads {
            builder = builder.app_context(uploads);
        }
        for (path, dir) in served_dirs {
            // Files the panel serves share its origin, so each directory route
            // is wrapped in the hardening layer that makes them inert
            // (GH #278) — the same path scopes the layer to that route only.
            builder = builder
                .layer(headers::ServedFileHeaders::new(&path))
                .serve_dir(route_path(&path), dir);
        }
        for page in pages {
            builder = builder.page(page);
        }
        for route in routes {
            builder = builder.route(route);
        }
        // The panel root has no home page of its own; until custom pages exist,
        // the prefix serves a redirect to the first resource's
        // list so the mount point is never a dead URL.
        if let Some(target) = root_target {
            builder = builder
                .app_context(RootRedirect(target))
                .route(RouteFn::new(
                    http::Method::GET,
                    route_path(&prefix),
                    panel_root_redirect,
                ));
        }
        #[cfg(feature = "auth")]
        {
            builder = builder.app_context(auth);
            if let Some(hint) = login_hint {
                builder = builder.app_context(LoginHint(hint));
            }
        }
        // The runtime layer registers last, outside every other pathless
        // layer: a page re-run is a marked POST the layer rewrites into a
        // GET for the page's own URL, and the layers it wraps must receive
        // the rewritten GET rather than the discarded POST.
        Ok(builder.runtime().build())
    }
}

impl Panel {
    /// Derive a [`NavigationItem`] for `R` using this panel's mount prefix.
    ///
    /// The one panel-aware navigation seam for a [`Resource`]:
    /// [`Panel::resource`](Self::resource) calls it, so a resource's
    /// [`Resource::navigation`] override reaches the sidebar instead of being
    /// dead API (GH #165). The override owns the **label, ordering and
    /// grouping**; the panel owns the **URL**, because it is the only party
    /// that knows where the resource is mounted. Concretely: an explicit
    /// [`NavTarget::Url`] in `R::navigation()` is taken as returned, and only
    /// a [`NavTarget::Derived`] target — the default, which names no URL
    /// because [`Resource::navigation`] takes no prefix — is resolved to
    /// `{prefix}/{slug}`.
    ///
    /// So `Panel::new("backoffice")` yields `"/backoffice/{slug}"` — never a
    /// hard-coded `"/admin"` — for default and overridden items alike, a URL an
    /// override spelled out stays exactly as written, and an override's `order`
    /// decides sidebar order (GH #102).
    pub(crate) fn nav_item<R: Resource>(&self) -> NavigationItem {
        R::navigation().resolved(&self.prefix, &R::slug())
    }
}

/// Whether a path is a route pattern ending in a catch-all, which is the only
/// shape [`DirectoryRoute`](topcoat::router::DirectoryRoute) accepts (GH #188).
///
/// Checked where the path is declared rather than where it is used: upstream
/// `serve_dir` panics on anything else, and `Panel::build` reports instead of
/// panicking (GH #174) — but the path comes from the app, and it would panic
/// first in [`route_path`] (which refuses to spell a route it cannot parse) and
/// then inside `DirectoryRoute::new` (which needs the catch-all last). Asking
/// both conditions here turns a typo into a build error instead of a panic
/// during the build.
fn is_directory_pattern(path: &str) -> bool {
    Path::from_str(path)
        .ok()
        .and_then(|parsed| parsed.segments().next_back())
        .is_some_and(|segment| segment.as_catch_all().is_some())
}

/// Validate one path segment a panel derives routes from (GH #174): a
/// `Resource::slug()` override, or a segment of the panel prefix.
///
/// Both reach a route path and, through the panel, a response body. A hostile
/// value — quote, backslash, CR/LF, `..`, slash, URL punctuation, a route
/// pattern character — must fail at registration rather than at request time,
/// so this is the export filename sanitizer's rule tightened to what a URL
/// segment can be: the export drops the offending characters because it must
/// still produce a download, while a route has no meaningful fallback.
///
/// The route pattern characters that a literal segment cannot carry (`{`, `}`,
/// `(`, `)`) are rejected rather than escaped (GH #295): `Path::from_str` treats
/// `{`/`(` as the start of a parameter or group segment, so a balanced pair
/// silently becomes a pattern and an unbalanced one panics [`route_path`]. `*`
/// stays accepted — it is a literal in a static segment — and the catch-all
/// spelling `{*name}` needs the `{` this rule already refuses.
fn validate_route_segment(kind: &str, segment: &str) -> Result<(), String> {
    if segment.is_empty() {
        return Err(format!("{kind}: path segment must not be empty"));
    }
    if segment == "." || segment == ".." {
        return Err(format!(
            "{kind} '{segment}': a path segment may not be '.' or '..'"
        ));
    }
    if let Some(bad) = segment.chars().find(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(
                c,
                '"' | '\\' | '/' | '?' | '#' | '%' | '&' | '=' | '{' | '}' | '(' | ')'
            )
    }) {
        return Err(format!(
            "{kind} '{segment}': a path segment may not contain {bad:?} (quotes, backslashes, control characters, whitespace, URL punctuation and the route pattern characters '{{', '}}', '(' and ')' are rejected)"
        ));
    }
    Ok(())
}

/// A resource's build-time declaration check (GH #138): monomorphized once per
/// declared resource by [`Panel::resource`], run by [`Panel::build`] with the
/// app's values and no request.
type ResourceCheck = fn(&Cx) -> Result<(), String>;

/// What a declared resource must be able to promise before the panel serves
/// it (GH #138).
///
/// The trait ships every method with a default, so a resource that overrides
/// nothing compiles and only fails when a user reaches a page. The essentials
/// that are *declarations* — a tenant predicate for a gated resource, a
/// renderable table, a form for the create page, a backed `unique()` marker —
/// are checked here, at build, and reported with the resource's type name.
/// Runtime essentials (the record fns) keep their existing loud failure: a
/// default stub answers "not implemented for <type>", never silently.
///
/// A declaration that panics is a boot failure too (GH #207): `Resource::table`
/// and `Resource::form` run code that panics on a mis-declaration (a duplicate
/// column name, a traversal lens), and this check's contract is a registration
/// error the caller can log or exit on. The whole body is caught — not just
/// those two calls — because `R::Model::schema()` and the policy predicates are
/// part of the same declaration, and a panic from any of them would otherwise
/// escape `build`; the check reports "declaring itself" rather than naming a
/// call it cannot attribute the panic to. The panic's own message is carried
/// into the error, so the cause survives a harness that installs its own hook.
///
/// `AssertUnwindSafe` is sound here because nothing observes the captured state
/// after an unwind: `cx` is the build-time [`validation_cx`] — an app context
/// holding the `Db` handle, owned by this call — and the panic fails the whole
/// `build`, so no request is ever served from it.
fn check_resource<R: Resource>(cx: &Cx) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_resource_inner::<R>(cx)
    })) {
        Ok(result) => result,
        Err(payload) => Err(format!(
            "resource `{}` panicked while declaring itself: {}",
            std::any::type_name::<R>(),
            panic_message(payload.as_ref())
        )),
    }
}

/// The message out of a caught panic payload (GH #207).
///
/// The declaration panics this catches are `assert!`/`panic!("…")` with a
/// formatted string, so `&str` and `String` cover every one of them; anything
/// else is reported by shape rather than silently dropped.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "a non-string panic payload".to_string()
    }
}

/// The body of [`check_resource`], unwound through `catch_unwind` so a
/// mis-declared resource is a registration error rather than a boot panic.
fn check_resource_inner<R: Resource>(cx: &Cx) -> Result<(), String> {
    // A gated resource that supplies no tenant predicate is misdeclared, and
    // the declaration is checkable without a request (GH #231):
    // `R::tenant_scope` is pure, and the default derivation answers by the
    // model's *shape* — a `tenant_id` UUID field, found by name and type — not
    // by the tenant value, so the nil UUID is enough to ask whether a predicate
    // exists at all. Refusing here is what `build`'s contract promises a
    // declaration error gets; the request-time error in `apply_tenant_scope`
    // stays as the backstop for a resource whose predicate is only `None` for
    // some tenants, and for app code that calls `scoped_query` outside a panel.
    //
    // Checked before the page essentials below because the gate and the scope
    // govern every handler this resource registers, not just the list and
    // create pages those checks are about.
    if R::requires_tenant() && R::tenant_scope(uuid::Uuid::nil()).is_none() {
        return Err(format!(
            "resource `{}` requires a tenant, but the framework cannot scope it: `{}` declares no \
             `tenant_id` UUID column to derive the filter from, and the resource does not override \
             `tenant_scope` — declare the column, override `tenant_scope`, or drop \
             `requires_tenant` and scope in `query` (GH #231)",
            std::any::type_name::<R>(),
            std::any::type_name::<R::Model>(),
        ));
    }
    // Chrome is attached by `wire_table_actions`, not by `R::table(cx)`
    // (GH #207): the record-key requirement is only knowable from the same
    // derivation the wiring reads.
    let chrome = declared_chrome::<R>(cx);
    if let Some(missing) = R::table(cx).missing_essentials(chrome) {
        return Err(format!(
            "resource `{}` cannot serve its list: {missing}",
            std::any::type_name::<R>()
        ));
    }
    // The form is only required where the panel would serve one, and `create`
    // is the statically checkable half of that (`can_update` needs a record).
    // The default policy denies create, so a read-only resource is unaffected.
    let form = R::form(cx);
    if R::can_create(cx) && form.is_empty() {
        return Err(format!(
            "resource `{}` allows create but its form declares no fields — build it with Schema::new(..)",
            std::any::type_name::<R>()
        ));
    }
    // `.unique()` is a promise the panel makes and the database has to keep
    // (GH #189 item 3): the marker turns the app-side pre-check on, so a field
    // whose column carries no unique index makes the panel enforce a rule
    // nothing else does — a duplicate the check lets through, or a rule the
    // database never asked for. The declaration checks are the only place both
    // halves are reachable without a request, so the pair is refused here
    // rather than discovered by a user. It is checked whatever the policies say:
    // a `unique()` marker is wrong on a form the panel would not even serve.
    // `lens_field_unique` recognizes composite indexes too, which is what makes
    // `#[unique(tenant_id, email)]` — the tenant-scoped arrangement the panel
    // documents — pass.
    let model = R::Model::schema();
    let root = model.as_root_unwrap();
    for (name, input) in form.text_inputs() {
        if !input.is_unique() {
            continue;
        }
        // A bound lens always resolves, so a name with no field at all is a
        // mis-declared schema — but it is not worth a second error string: it
        // fails the same way, one message below.
        let backed = root
            .fields
            .iter()
            .filter(|field| field.name.app_unwrap() == name)
            .any(|field| crate::schema::lens_field_unique(field, root));
        if !backed {
            return Err(format!(
                "resource `{}` marks form field `{name}` unique, but `{}::{name}` carries no unique index — add `#[unique]` (or `#[unique(..)]`) to the column or drop `.unique()`, which would otherwise check a rule the database does not enforce",
                std::any::type_name::<R>(),
                std::any::type_name::<R::Model>()
            ));
        }
    }
    Ok(())
}

/// A context for the build-time declaration checks: the app's own values, no
/// request. Resources must be able to describe their table and form from this
/// — that they cannot read a request here is the contract, not a limitation.
fn validation_cx(db: &Db) -> Cx {
    let mut app_context = topcoat::context::AppContext::new();
    app_context.insert(db.clone());
    Cx::new(std::sync::Arc::new(app_context))
}

/// Parse a panel route path, panicking on malformed input — the paths are
/// built from the panel prefix and the resource slug, both validated at
/// registration ([`validate_route_segment`]), so a malformed path here is a
/// framework bug rather than user input.
pub(crate) fn route_path(path: &str) -> topcoat::router::PathBuf {
    Path::from_str(path)
        .expect("panel route paths are well-formed")
        .to_owned()
}

/// Defense-in-depth companion to the auth gate (GH #130, ADR-0013): every
/// panel handler and the live-search shard re-check the resolved user, so a
/// missing or mis-mounted gate cannot silently open a handler. A no-op when
/// the panel explicitly disabled auth.
#[cfg(feature = "auth")]
pub(crate) fn enforce_auth(cx: &Cx) -> Result<(), topcoat::Error> {
    if crate::auth::enforced(cx) {
        crate::auth::require_authenticated(cx)?;
    }
    Ok(())
}

/// Auth compiled out: the gate does not exist either, so nothing to enforce.
#[cfg(not(feature = "auth"))]
pub(crate) fn enforce_auth(_cx: &Cx) -> Result<(), topcoat::Error> {
    Ok(())
}

/// Enforce tenancy gating for resources that require it (GH #87).
///
/// Wired into every resource handler; a no-op unless the resource overrides
/// `Resource::requires_tenant`. Fails closed (403) when no tenant is present
/// instead of serving unscoped rows.
pub(crate) fn enforce_tenant<R: Resource>(cx: &Cx) -> Result<(), topcoat::Error> {
    if R::requires_tenant() {
        crate::tenancy::require_tenant(cx)?;
    }
    Ok(())
}

/// The gate every resource handler runs first: the authenticated user, then
/// the resource's tenant. A no-op when auth is compiled out and the resource
/// declares no tenant.
pub(crate) fn gate<R: Resource>(cx: &Cx) -> Result<(), topcoat::Error> {
    enforce_auth(cx)?;
    enforce_tenant::<R>(cx)
}

/// The panel's URL prefix: the [`PanelPrefix`] app context installed by
/// [`Panel::build`], else the request path's first segment, else `/admin`.
///
/// A bare `CxTestBuilder` installs no prefix, so a test rendering under
/// `/admin/...` still derives `/admin`.
pub(crate) fn panel_prefix(cx: &Cx) -> String {
    topcoat::context::try_app_context::<PanelPrefix>(cx)
        .map(|p| p.0.clone())
        .unwrap_or_else(|| {
            let path = topcoat::router::request::uri(cx).path().to_string();
            path.split('/')
                .nth(1)
                .filter(|s| !s.is_empty())
                .map(|s| format!("/{s}"))
                .unwrap_or_else(|| "/admin".to_string())
        })
}

/// The list URL for a resource: `{panel prefix}/{slug}`.
pub(crate) fn list_url(cx: &Cx, slug: &str) -> String {
    format!("{}/{slug}", panel_prefix(cx))
}

/// The panel root: a temporary redirect to the first declared resource's
/// list, so the mount point is never a dead URL (custom pages remain future
/// work; see `docs/guide/src/panel-and-routing.md`). Filament registers its
/// home page here.
pub(crate) fn panel_root_redirect(cx: &Cx, _body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        // Defense in depth (GH #146): every panel handler re-checks the
        // resolved user, so a missing or mis-mounted gate cannot leak the
        // first resource's slug via the redirect target.
        enforce_auth(cx)?;
        let RootRedirect(target) = app_context::<RootRedirect>(cx);
        Err(redirect(target.clone()).into())
    })
}

#[cfg(test)]
mod tests {
    use toasty::Db;

    use super::*;
    use crate::panel::test_support::{Dummy, dummy_table, panel_for};
    /// GH #188: a served directory's path is a route pattern ending in a
    /// catch-all, and only that; everything else is a build error rather than
    /// the panic upstream `serve_dir` would raise.
    #[test]
    fn serve_dir_accepts_only_a_catch_all_pattern() {
        assert!(is_directory_pattern("/uploads/{*file}"));
        assert!(is_directory_pattern("/{*file}"));
        // Upstream allows a space in a static segment, so a pattern carrying
        // one is still a pattern: the catch-all is what matters, not tidiness.
        assert!(is_directory_pattern("/up loads/{*file}"));
        // Not a catch-all: a plain path, its trailing-slash form, a named
        // parameter, an unnamed catch-all, and a catch-all that is not last.
        assert!(!is_directory_pattern("/uploads"));
        assert!(!is_directory_pattern("/uploads/"));
        assert!(!is_directory_pattern("/uploads/{file}"));
        assert!(!is_directory_pattern("/uploads/{*}"));
        assert!(!is_directory_pattern("/{*file}/more"));
        // Not a route path at all: an unclosed brace, an empty segment, and a
        // catch-all name that is not an identifier.
        assert!(!is_directory_pattern("/uploads/{*file"));
        assert!(!is_directory_pattern("/uploads//{*file}"));
        assert!(!is_directory_pattern("/uploads/{*fi-le}"));
    }

    #[test]
    fn panel_normalizes_prefix() {
        assert_eq!(Panel::new("admin").prefix(), "/admin");
        assert_eq!(Panel::new("/admin").prefix(), "/admin");
        assert_eq!(Panel::new("admin/").prefix(), "/admin");
        assert_eq!(Panel::new("/admin/").prefix(), "/admin");
        assert_eq!(Panel::new("").prefix(), "/admin");
    }

    #[test]
    fn list_url_prefers_panel_prefix_over_request_path() {
        use topcoat::context::CxTestBuilder;

        // With the panel prefix installed, the resource URL is derived from
        // the declaration — even on a path that is not the list route.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users/42/edit")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(PanelPrefix("/admin".to_string()))
            .build();
        assert_eq!(list_url(&cx, "users"), "/admin/users");

        let (parts, ()) = http::Request::builder()
            .uri("/backoffice/users")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(PanelPrefix("/backoffice".to_string()))
            .build();
        assert_eq!(list_url(&cx, "users"), "/backoffice/users");

        // Without a panel prefix (bare test builder), fall back to the
        // request path's first segment.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users/42/edit")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert_eq!(list_url(&cx, "users"), "/admin/users");
    }

    #[tokio::test]
    async fn panel_mounts_runtime_page_rerun_routes() {
        use crate::resource::Resource;

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let db = Db::builder().connect("sqlite::memory:").await.unwrap();
        let router = panel_for::<DummyResource>(db)
            .build()
            .expect("panel builds");

        // The list page denies by default (default-deny policy → 403). A
        // POST carrying the runtime marker rewrites into a GET for the
        // page's own URL, so it reaches the handler and reports 403;
        // without `.runtime()` on the builder the marked POST never becomes
        // a page GET. (Topcoat's `runtime::script` requires this layer.)
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("/admin/dummies")
            .header("content-type", "application/json")
            .header(&topcoat::runtime::RUNTIME_HEADER, "true")
            .body(Body::from("{}".to_owned()))
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
    }

    /// GH #102: `Panel::dark_mode` is the theme a first-time visitor gets. It
    /// must reach the rendered document's `<html class>`.
    #[cfg(feature = "auth")]
    #[tokio::test]
    async fn dark_mode_sets_the_document_class() {
        let db = Db::builder()
            .models(toasty::models!(
                crate::auth::AdminUser,
                crate::auth::AuthSession
            ))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .auth(crate::Auth::password())
            .dark_mode(true)
            .build()
            .expect("panel builds");

        // The standalone login page renders the same document the admin shell
        // does (ADR-0013), so it carries the theme class without a session.
        let response = router
            .handle(
                http::Request::builder()
                    .uri("/admin/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), http::StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let html = String::from_utf8_lossy(&bytes);
        assert!(
            html.contains("<html class=\"dark\">"),
            "dark_mode(true) must set the document's dark class, got {html}"
        );
    }

    /// The panel root answers the gate before reading `RootRedirect`
    /// (GH #146 defense in depth): a mis-mounted gate must not leak the
    /// first resource's slug via the redirect target.
    #[cfg(feature = "auth")]
    #[tokio::test]
    async fn panel_root_redirect_rechecks_auth_before_the_root_target() {
        use topcoat::{context::CxTestBuilder, router::response::IntoResponse};

        // Enforced auth, no resolved user: the handler itself redirects to
        // login — and never reaches the `RootRedirect` read (absent here, so
        // a missing re-check would panic instead of answering).
        let (parts, ()) = http::Request::builder()
            .uri("/admin")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(crate::Auth::password())
            .build();
        let err = match panel_root_redirect(&cx, Body::empty()).await {
            Ok(_) => panic!("unauthenticated root must not read RootRedirect"),
            Err(err) => err,
        };
        let location = err
            .into_response(&cx)
            .expect("gate redirect renders")
            .headers()
            .get(http::header::LOCATION)
            .expect("login redirect carries a location")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            location.starts_with("/admin/login"),
            "unauthenticated root must redirect to login, got {location}"
        );

        // A resolved user passes the re-check and lands on the first resource.
        let user = crate::auth::CurrentUser {
            id: "u1".to_string(),
            login: "ada@example.com".to_string(),
            display_name: "Ada".to_string(),
            tenant_id: None,
            can_access_panel: true,
        };
        let (parts, ()) = http::Request::builder()
            .uri("/admin")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(crate::Auth::password())
            .app_context(RootRedirect("/admin/users".to_string()))
            .request_context(user)
            .build();
        let err = match panel_root_redirect(&cx, Body::empty()).await {
            Ok(_) => panic!("the redirect is an Err response"),
            Err(err) => err,
        };
        let location = err
            .into_response(&cx)
            .expect("root redirect renders")
            .headers()
            .get(http::header::LOCATION)
            .expect("root redirect carries a location")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(location, "/admin/users");
    }

    /// The named runtime endpoints answer the gate (GH #146): a request
    /// without a session to a shard's fixed path is refused with 401, not a
    /// login redirect and not the shard's content. The path is stable, so the
    /// refusal is the only thing that keeps it from being probed.
    #[cfg(feature = "auth")]
    #[tokio::test]
    async fn named_shard_endpoints_answer_401_without_a_session() {
        let db = Db::builder()
            .models(toasty::models!(
                crate::auth::AdminUser,
                crate::auth::AuthSession
            ))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .auth(crate::Auth::password())
            .build()
            .expect("panel builds");

        for path in [TABLE_SEARCH_PATH, crate::notification::LIVE_TOASTER_PATH] {
            let request = http::Request::builder()
                .method(http::Method::POST)
                .uri(path)
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}".to_owned()))
                .unwrap();
            let response = router.handle(request).await;
            assert_eq!(
                response.status(),
                http::StatusCode::UNAUTHORIZED,
                "an unauthenticated shard request must answer 401: {path}"
            );
        }
    }

    /// GH #174: a panel with no `Db` is a configuration error, not a panic.
    #[test]
    fn panel_build_errors_without_db() {
        // `Router` has no `Debug`, so `expect_err` cannot report the Ok case.
        let Err(error) = Panel::new("admin").build() else {
            panic!("a panel without a Db must not build");
        };
        assert!(
            format!("{error}").contains("requires a Db"),
            "the error must name the missing Db, got {error}"
        );
    }

    /// The feature-off build has no gate, so it refuses a panel that has not
    /// acknowledged that (ADR-0013): serving ungated stays a line of app code,
    /// never a side effect of trimming dependencies.
    #[cfg(not(feature = "auth"))]
    #[tokio::test]
    async fn build_refuses_an_unacknowledged_ungated_panel() {
        use crate::resource::Resource;

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;

            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let Err(error) = Panel::new("admin")
            .app_context(db)
            .resource::<DummyResource>()
            .build()
        else {
            panic!("an ungated panel must not build without the auth feature");
        };
        assert!(
            format!("{error}").contains("auth"),
            "the error must name the missing auth feature, got {error}"
        );
    }

    /// The explicit opt-out is the acknowledgement `build` requires, so an app
    /// that asks for an ungated panel gets one.
    #[cfg(not(feature = "auth"))]
    #[tokio::test]
    async fn build_accepts_the_explicit_opt_out() {
        use crate::resource::Resource;

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;

            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        panel_for::<DummyResource>(db)
            .build()
            .expect("the explicit opt-out builds the panel");
    }

    /// CSRF does not depend on the `auth` feature (GH #99): with the gate
    /// compiled out, a create POST without a matching `csrf_token` is still
    /// 403, so dropping sessions does not drop the double-submit check.
    #[cfg(not(feature = "auth"))]
    #[tokio::test]
    async fn csrf_is_enforced_without_the_auth_feature() {
        use crate::{
            resource::Resource,
            schema::{Schema, TextInput},
        };

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;

            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
            fn form(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Dummy::fields().name()))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = panel_for::<DummyResource>(db)
            .build()
            .expect("the explicit opt-out builds the panel");

        let response = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri("/admin/dummies/create")
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(Body::from("name=Ada"))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            response.status(),
            http::StatusCode::FORBIDDEN,
            "a create POST with no csrf_token must fail closed"
        );
    }

    /// GH #176: every page the panel renders carries the clickjacking
    /// directive by default, and both escape hatches work — a deployment
    /// directive, and an opt-out for a proxy that owns the whole policy.
    #[tokio::test]
    async fn panel_sends_frame_ancestors_unless_opted_out() {
        use crate::resource::Resource;

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;

            // A rendered page, not the default-deny 403: an error response is
            // produced above the layer chain, so only a served document proves
            // the header is installed.
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        /// The directive the finished page carries.
        async fn policy(panel: Panel) -> Option<String> {
            let router = panel.build().expect("panel builds");
            let response = router
                .handle(
                    http::Request::builder()
                        .uri("/admin/dummies")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(response.status(), http::StatusCode::OK, "page must render");
            response
                .headers()
                .get(http::header::CONTENT_SECURITY_POLICY)
                .map(|value| value.to_str().unwrap().to_string())
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let base = || panel_for::<DummyResource>(db.clone());

        assert_eq!(
            policy(base()).await.as_deref(),
            Some("frame-ancestors 'self'"),
            "a panel page must not be frameable by default"
        );
        assert_eq!(
            policy(base().frame_ancestors("'self' https://intranet.example"))
                .await
                .as_deref(),
            Some("frame-ancestors 'self' https://intranet.example"),
            "a deployment that frames the panel says so"
        );
        assert!(
            policy(base().without_frame_ancestors()).await.is_none(),
            "an opted-out panel sends no policy of its own"
        );
    }

    #[test]
    fn panel_navigation_item_respects_prefix() {
        use crate::resource::Resource;

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
        }

        let panel = Panel::new("backoffice");
        let item = panel.nav_item::<DummyResource>();
        // Label: pluralized model name ("Dummy" → "Dummies"); URL: prefix +
        // resource slug ("DummyResource" → "dummies"), resolved by the panel.
        assert_eq!(item.label, "Dummies");
        assert_eq!(item.url(), Some("/backoffice/dummies"));

        let default = Panel::new("admin").nav_item::<DummyResource>();
        assert_eq!(default.url(), Some("/admin/dummies"));
        // Mount normalisation is `Panel::new`'s (slashes trimmed, `/admin` when
        // empty), and the resolved URL follows it.
        let slashed = Panel::new("/backoffice/").nav_item::<DummyResource>();
        assert_eq!(slashed.url(), Some("/backoffice/dummies"));
        let bare = Panel::new("").nav_item::<DummyResource>();
        assert_eq!(bare.url(), Some("/admin/dummies"));
    }

    /// GH #165: `Resource::navigation()` reaches the sidebar, and its order is
    /// what the rendered shell sorts by.
    #[test]
    fn panel_navigation_item_honours_override_order_with_prefix_adjusted_url() {
        use crate::resource::{NavigationItem, Resource};

        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;

            fn navigation() -> NavigationItem {
                // The override cannot know the panel prefix, so it decorates
                // the default item: order here, URL from the panel.
                NavigationItem {
                    order: -1,
                    ..NavigationItem::for_resource::<Self>()
                }
            }
        }
        struct PlainResource;
        impl Resource for PlainResource {
            type Model = Dummy;

            fn slug() -> String {
                "plain".to_string()
            }
        }

        // Non-`/admin` panel + overridden navigation: the order survives and
        // the URL is resolved under this panel's prefix, not `/admin`.
        let panel = Panel::new("backoffice");
        let item = panel.nav_item::<DummyResource>();
        assert_eq!(item.order, -1);
        assert_eq!(item.label, "Dummies");
        assert_eq!(item.url(), Some("/backoffice/dummies"));
        // A resource without an override keeps the default (declaration order).
        assert_eq!(panel.nav_item::<PlainResource>().order, 0);
    }

    /// GH #165: the override reaches *rendered* sidebar order.
    /// Rendered on a non-`/admin` panel, so the same test also pins the URL
    /// half: the sidebar links under `/backoffice`, never the origin `/admin`.
    #[tokio::test]
    async fn panel_sidebar_renders_overridden_navigation_order_first() {
        use topcoat::{
            context::CxTestBuilder,
            view::{ViewExt, view},
        };

        use crate::resource::{NavigationItem, Resource};

        struct PinnedResource;
        impl Resource for PinnedResource {
            type Model = Dummy;

            fn slug() -> String {
                "pinned".to_string()
            }

            fn navigation() -> NavigationItem {
                // GH #165 regression shape: an override that only sets order.
                // Before the fix the sidebar kept declaration order and the
                // resource's `order: -1` had no effect at all.
                NavigationItem {
                    order: -1,
                    ..NavigationItem::for_resource::<Self>()
                }
            }
        }
        struct OtherResource;
        impl Resource for OtherResource {
            type Model = Dummy;

            fn slug() -> String {
                "other".to_string()
            }

            fn navigation_label() -> String {
                "Other".to_string()
            }
        }

        // `PinnedResource` is declared last, so only the override can move it up.
        let panel = Panel::new("backoffice");
        let nav_items = vec![
            panel.nav_item::<OtherResource>(),
            panel.nav_item::<PinnedResource>(),
        ];
        let (parts, ()) = http::Request::builder()
            .uri("/backoffice/other")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &nav_items, "/backoffice/other", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let pinned_at = html
            .find("/backoffice/pinned")
            .unwrap_or_else(|| panic!("pinned item must link under the panel prefix, got {html}"));
        let other_at = html.find("/backoffice/other").expect("other item renders");
        assert!(
            !html.contains("/admin/pinned"),
            "navigation must not link at the origin mount, got {html}"
        );
        assert!(
            pinned_at < other_at,
            "an overridden order: -1 must render first, got {html}"
        );
    }

    /// GH #165: a URL an override spells out is the author's, not the panel's —
    /// only a `Derived` target is resolved. A cross-panel link, a query view, or
    /// a custom path segment must survive untouched on a non-`/admin` panel,
    /// *including* one that looks like the origin mount.
    #[test]
    fn panel_navigation_item_keeps_urls_the_override_spells_out() {
        use crate::resource::{NavTarget, NavigationItem, Resource};

        struct DraftsResource;
        impl Resource for DraftsResource {
            type Model = Dummy;

            fn slug() -> String {
                "drafts".to_string()
            }

            fn navigation_label() -> String {
                "Drafts".to_string()
            }

            fn navigation() -> NavigationItem {
                // Order only, no URL: still the panel's to resolve.
                NavigationItem {
                    order: 3,
                    ..NavigationItem::for_resource::<Self>()
                }
            }
        }

        // `label`/`order` decorate the default item without touching its URL,
        // so the panel still owns (and resolves) the URL.
        let decorated = Panel::new("backoffice").nav_item::<DraftsResource>();
        assert_eq!(decorated.label, "Drafts");
        assert_eq!(decorated.order, 3);
        assert_eq!(decorated.url(), Some("/backoffice/drafts"));

        // A spelled-out URL is left alone — `/admin/posts?…` on a `/backoffice`
        // panel is a deliberate link, not a stale mount.
        struct ReportsResource;
        impl Resource for ReportsResource {
            type Model = Dummy;

            fn slug() -> String {
                "reports".to_string()
            }

            fn navigation() -> NavigationItem {
                NavigationItem::at("Draft posts", "/admin/posts?filters=status:draft")
            }
        }
        let spelled_out = Panel::new("backoffice").nav_item::<ReportsResource>();
        assert_eq!(spelled_out.url(), Some("/admin/posts?filters=status:draft"));
        assert_eq!(spelled_out.label, "Draft posts");
        assert!(matches!(spelled_out.target, NavTarget::Url(_)));

        // The same URL spelled out on the resource's *own* slug is the author's
        // too: `Derived` is what the Panel resolves, never a URL that happens to
        // match the origin mount.
        struct OwnSlugResource;
        impl Resource for OwnSlugResource {
            type Model = Dummy;

            fn slug() -> String {
                "users".to_string()
            }

            fn navigation() -> NavigationItem {
                NavigationItem::at("Users (legacy)", "/admin/users")
            }
        }
        let own_slug = Panel::new("backoffice").nav_item::<OwnSlugResource>();
        assert_eq!(own_slug.url(), Some("/admin/users"));
    }

    #[test]
    fn panel_navigation_items_are_distinct_for_multiple_resources() {
        use crate::resource::Resource;

        struct UserResource;
        impl Resource for UserResource {
            type Model = Dummy;
        }
        struct CategoryResource;
        impl Resource for CategoryResource {
            type Model = Dummy;

            fn slug() -> String {
                "categories".to_string()
            }

            fn navigation_label() -> String {
                "Categories".to_string()
            }
        }

        let panel = Panel::new("admin");
        let users = panel.nav_item::<UserResource>();
        let categories = panel.nav_item::<CategoryResource>();
        assert_eq!(users.url(), Some("/admin/users"));
        assert_eq!(categories.url(), Some("/admin/categories"));
        assert_ne!(users.url(), categories.url());
    }

    /// GH #174: duplicate slugs are reported by `build`, not asserted in the
    /// declarative builder — a panel is configured, then validated once.
    #[test]
    fn panel_build_rejects_duplicate_resource_slugs() {
        use crate::resource::Resource;

        struct FirstResource;
        impl Resource for FirstResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
        }
        struct SecondResource;
        impl Resource for SecondResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
        }

        let Err(error) = Panel::new("admin")
            .resource::<FirstResource>()
            .resource::<SecondResource>()
            .build()
        else {
            panic!("two resources over one slug must not build");
        };
        assert!(
            format!("{error}").contains("duplicate resource slug"),
            "the error must name the duplicate, got {error}"
        );
    }

    /// GH #174: `slug()` is free-form and reaches route paths and response
    /// headers, so a hostile value fails registration instead of splitting a
    /// header or panicking in `route_path` at boot.
    #[test]
    fn panel_build_rejects_a_hostile_slug() {
        use crate::resource::Resource;

        struct HostileResource;
        impl Resource for HostileResource {
            type Model = Dummy;

            fn slug() -> String {
                "a\"b\r\n".to_string()
            }
        }

        let Err(error) = Panel::new("admin").resource::<HostileResource>().build() else {
            panic!("a slug with quotes and CRLF must not build");
        };
        assert!(
            format!("{error}").contains("Resource::slug"),
            "the error must name the offending slug, got {error}"
        );
    }

    /// GH #174/#295: a slug carrying a route pattern character a literal segment
    /// cannot hold is a declared registration error, not a panic in
    /// [`route_path`]. `Path::from_str` starts a parameter segment at `{` and a
    /// group at `(`, so an unbalanced pair panics the route builder and a
    /// balanced one silently makes the slug a pattern.
    #[test]
    fn panel_build_rejects_route_pattern_characters_in_a_slug() {
        use crate::resource::Resource;

        macro_rules! pattern_resource {
            ($name:ident, $slug:literal) => {
                struct $name;
                impl Resource for $name {
                    type Model = Dummy;
                    fn slug() -> String {
                        $slug.to_string()
                    }
                }
            };
        }
        pattern_resource!(BraceOpen, "a{b");
        pattern_resource!(BraceClose, "a}b");
        pattern_resource!(ParenOpen, "a(b");
        pattern_resource!(ParenClose, "a)b");

        macro_rules! rejects {
            ($name:ident, $slug:literal) => {{
                let Err(error) = Panel::new("admin").resource::<$name>().build() else {
                    panic!("a slug containing {} must not build", $slug);
                };
                let error = format!("{error}");
                assert!(
                    error.contains("Resource::slug") && error.contains($slug),
                    "the error must name the offending slug {:?}, got {error}",
                    $slug
                );
            }};
        }
        rejects!(BraceOpen, "a{b");
        rejects!(BraceClose, "a}b");
        rejects!(ParenOpen, "a(b");
        rejects!(ParenClose, "a)b");

        // The prefix goes through the same rule, once per segment.
        let Err(error) = Panel::new("adm{in}").build() else {
            panic!("a panel prefix with a route pattern character must not build");
        };
        assert!(
            format!("{error}").contains("panel prefix"),
            "the error must name the panel prefix, got {error}"
        );
    }

    /// A slug made of ordinary URL-segment characters still builds, and its
    /// list route resolves (GH #295): rejecting the pattern characters must not
    /// reject the accepted ones.
    #[tokio::test]
    async fn a_plain_slug_builds_and_resolves() {
        use crate::resource::Resource;

        struct PlainResource;
        impl Resource for PlainResource {
            type Model = Dummy;
            fn slug() -> String {
                "user-profiles_2".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .paginate(25)
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = panel_for::<PlainResource>(db)
            .build()
            .expect("a plain slug builds");
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("/admin/user-profiles_2")
            .body(Body::empty())
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "the list route a plain slug builds must resolve"
        );
        let html = String::from_utf8_lossy(
            &http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes(),
        )
        .to_string();
        assert!(
            html.contains("Dummies</h1>"),
            "the resolved list page must render its title: {html}"
        );
    }

    /// A slug containing `*` builds and resolves (GH #295): `*` is a literal
    /// static segment in the router, so rejecting it would break a slug that
    /// worked; only the `{*name}` catch-all spelling carries meaning, and the
    /// `{` it needs is already refused.
    #[tokio::test]
    async fn a_star_slug_builds_and_resolves() {
        use crate::resource::Resource;

        struct StarResource;
        impl Resource for StarResource {
            type Model = Dummy;
            fn slug() -> String {
                "user*profiles".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .paginate(25)
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = panel_for::<StarResource>(db)
            .build()
            .expect("a slug containing `*` builds");
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("/admin/user*profiles")
            .body(Body::empty())
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "the list route a `*` slug builds must resolve"
        );
        let html = String::from_utf8_lossy(
            &http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes(),
        )
        .to_string();
        assert!(
            html.contains("Dummies</h1>"),
            "the resolved list page must render its title: {html}"
        );
    }

    /// GH #189 item 3: `.unique()` is a promise about the column, so declaring
    /// it on a field with no unique index fails the build instead of turning on
    /// a check the database does not back.
    #[tokio::test]
    async fn panel_build_rejects_a_unique_marker_without_a_unique_index() {
        use crate::{
            resource::{Resource, Table, TextColumn},
            schema::{Schema, TextInput},
        };

        #[derive(Debug, toasty::Model, Clone)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            nickname: String,
        }
        struct UnbackedResource;
        impl Resource for UnbackedResource {
            type Model = Subscriber;
            fn slug() -> String {
                "subscribers".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> Table<Subscriber> {
                Table::r#for(cx)
                    .id(|s: &Subscriber| s.id.to_string())
                    .columns(TextColumn::r#for(
                        Subscriber::fields().nickname(),
                        |s: &Subscriber| s.nickname.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Subscriber::fields().nickname()).unique())
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let Err(error) = Panel::new("admin")
            .app_context(db)
            .resource::<UnbackedResource>()
            .build()
        else {
            panic!("a `unique()` marker with no unique index must not build");
        };
        let error = format!("{error}");
        assert!(
            error.contains("`nickname`") && error.contains("no unique index"),
            "the error must name the field and the missing index, got {error}"
        );
    }

    /// The marker is a property of the declaration, not of the policy serving
    /// it (GH #189): a read-only resource — `can_create` denied, the default —
    /// still fails the build on an unbacked `unique()`, so fixing the policy
    /// later cannot silently re-arm a check the database does not keep.
    #[tokio::test]
    async fn panel_build_rejects_an_unbacked_unique_marker_even_when_create_is_denied() {
        use crate::{
            resource::{Resource, Table, TextColumn},
            schema::{Schema, TextInput},
        };

        #[derive(Debug, toasty::Model, Clone)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            nickname: String,
        }
        struct ReadOnlyResource;
        impl Resource for ReadOnlyResource {
            type Model = Subscriber;
            fn slug() -> String {
                "subscribers".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            // `can_create` keeps its default (deny); only the form is declared.
            fn table(cx: &Cx) -> Table<Subscriber> {
                Table::r#for(cx)
                    .id(|s: &Subscriber| s.id.to_string())
                    .columns(TextColumn::r#for(
                        Subscriber::fields().nickname(),
                        |s: &Subscriber| s.nickname.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Subscriber::fields().nickname()).unique())
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let Err(error) = Panel::new("admin")
            .app_context(db)
            .resource::<ReadOnlyResource>()
            .build()
        else {
            panic!("the marker is unbacked whether or not create is allowed");
        };
        assert!(
            format!("{error}").contains("no unique index"),
            "the error must be the marker's, not the policy's, got {error}"
        );
    }

    /// The guard's other half: a unique index — single-field or composite —
    /// keeps building. `lens_field_unique` reads the model's index list, so
    /// `#[unique(tenant_id, email)]` (the tenant-scoped arrangement the panel
    /// documents) is not a false positive.
    #[tokio::test]
    async fn panel_build_accepts_unique_markers_with_a_backing_index() {
        use crate::{
            resource::{Resource, Table, TextColumn},
            schema::{Schema, TextInput},
        };

        #[derive(Debug, toasty::Model, Clone)]
        #[unique(tenant_id, email)]
        struct Author {
            #[key]
            #[auto]
            id: uuid::Uuid,
            tenant_id: uuid::Uuid,
            email: String,
        }
        struct AuthorResource;
        impl Resource for AuthorResource {
            type Model = Author;
            fn slug() -> String {
                "authors".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> Table<Author> {
                Table::r#for(cx)
                    .id(|a: &Author| a.id.to_string())
                    .columns(TextColumn::r#for(Author::fields().email(), |a: &Author| {
                        a.email.clone()
                    }))
            }
            fn form(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Author::fields().email()).unique())
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Author))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        panel_for::<AuthorResource>(db)
            .build()
            .expect("a composite unique index backs the marker");
    }

    /// GH #207 part 1: `R::table(cx)` carries no action chrome —
    /// `wire_table_actions` attaches it — so the record-key requirement is only
    /// knowable from the same declaration the wiring reads. A resource with
    /// action chrome and no `.pk(..)` fails `build`.
    #[tokio::test]
    async fn panel_build_rejects_action_chrome_without_a_record_key() {
        use crate::{
            resource::{Resource, Table, TextColumn},
            schema::{Schema, TextInput},
        };

        #[derive(Debug, toasty::Model, Clone)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            nickname: String,
        }

        fn keyless_table(cx: &Cx) -> Table<Subscriber> {
            Table::r#for(cx)
                .id(|s: &Subscriber| s.id.to_string())
                .columns(TextColumn::r#for(
                    Subscriber::fields().nickname(),
                    |s: &Subscriber| s.nickname.clone(),
                ))
        }

        /// Chrome opted into explicitly (GH #226): the default opts out of
        /// both links, so a resource that wants them names them — and that is
        /// what makes the record key required.
        struct ChromeResource;
        impl Resource for ChromeResource {
            type Model = Subscriber;
            fn slug() -> String {
                "subscribers".to_string()
            }
            fn deletable() -> bool {
                true
            }
            fn editable() -> bool {
                true
            }
            fn table(cx: &Cx) -> Table<Subscriber> {
                keyless_table(cx)
            }
        }

        /// Chrome left at the opt-in default, so no record key is needed.
        struct ChromeOffResource;
        impl Resource for ChromeOffResource {
            type Model = Subscriber;
            fn slug() -> String {
                "subscribers".to_string()
            }
            fn table(cx: &Cx) -> Table<Subscriber> {
                keyless_table(cx)
            }
        }

        /// Delete and edit left at the opt-in default, but the detail page is
        /// declared (GH #187), so the View link is action chrome all the same.
        struct ViewedResource;
        impl Resource for ViewedResource {
            type Model = Subscriber;
            fn slug() -> String {
                "subscribers".to_string()
            }
            fn table(cx: &Cx) -> Table<Subscriber> {
                keyless_table(cx)
            }
            fn view(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Subscriber::fields().nickname()))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let panel = || {
            Panel::new("admin")
                .app_context(db.clone())
                .auth(crate::Auth::disabled())
        };

        let Err(error) = panel().resource::<ChromeResource>().build() else {
            panic!("action chrome without a record key must not build");
        };
        assert!(
            format!("{error}").contains("action chrome needs a record key"),
            "the error must name the missing record key, got {error}"
        );

        let Err(error) = panel().resource::<ViewedResource>().build() else {
            panic!("a View link is action chrome too");
        };
        assert!(
            format!("{error}").contains("action chrome needs a record key"),
            "the error must name the missing record key, got {error}"
        );

        panel()
            .resource::<ChromeOffResource>()
            .build()
            .expect("a resource with no action chrome needs no record key");
    }

    /// GH #207 part 2: `Resource::table` and `Resource::form` run code that
    /// panics on a mis-declaration, but `build`'s contract is a registration
    /// error the caller can log or exit on. Both classes below are caught at
    /// the boundary instead of unwinding out of `build`.
    #[tokio::test]
    async fn panel_build_turns_declaration_panics_into_registration_errors() {
        use crate::resource::{Resource, Table, TextColumn};

        #[derive(Debug, Clone, toasty::Embed)]
        struct Meta {
            note: String,
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            title: String,
            meta: Meta,
        }

        /// Two columns over one field: `Table::columns` asserts on the
        /// duplicate name (GH #156).
        struct DuplicateColumnResource;
        impl Resource for DuplicateColumnResource {
            type Model = Doc;
            fn slug() -> String {
                "docs".to_string()
            }
            fn table(cx: &Cx) -> Table<Doc> {
                Table::r#for(cx).id(|d: &Doc| d.id.to_string()).columns((
                    TextColumn::r#for(Doc::fields().title(), |d: &Doc| d.title.clone()),
                    TextColumn::r#for(Doc::fields().title(), |d: &Doc| d.title.clone()),
                ))
            }
        }

        /// An embedded step is not a single-field lens: `lens_field` refuses
        /// the traversal loudly (GH #100), which without the boundary catch is
        /// a boot panic.
        struct TraversalLensResource;
        impl Resource for TraversalLensResource {
            type Model = Doc;
            fn slug() -> String {
                "docs".to_string()
            }
            fn table(cx: &Cx) -> Table<Doc> {
                Table::r#for(cx)
                    .id(|d: &Doc| d.id.to_string())
                    .columns(TextColumn::r#for(Doc::fields().meta().note(), |d: &Doc| {
                        d.meta.note.clone()
                    }))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Doc))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let panel = || {
            Panel::new("admin")
                .app_context(db.clone())
                .auth(crate::Auth::disabled())
        };

        let Err(error) = panel().resource::<DuplicateColumnResource>().build() else {
            panic!("a duplicate column name must not build");
        };
        let error = format!("{error}");
        assert!(
            error.contains("panicked while declaring") && error.contains("duplicate column name"),
            "the panic's own message must survive into the registration error, got {error}"
        );

        let Err(error) = panel().resource::<TraversalLensResource>().build() else {
            panic!("a traversal lens must not build");
        };
        let error = format!("{error}");
        assert!(
            error.contains("panicked while declaring") && error.contains("single-field lens"),
            "the panic's own message must survive into the registration error, got {error}"
        );
    }

    /// GH #231: a gated resource that supplies no tenant predicate is a
    /// declaration error, and #223's `tenant_scope` probe is pure — so `build`
    /// refuses it with an error naming the resource instead of waiting for the
    /// first request to answer its logged 500. The override half builds, so the
    /// check rejects a *missing* predicate rather than the hook itself.
    #[tokio::test]
    async fn panel_build_rejects_a_gated_resource_with_no_tenant_predicate() {
        use crate::resource::{Resource, Table, TextColumn};

        /// A renderable table, so tenancy is the *only* thing either resource
        /// below could be refused for: the rejection is the tenant probe's, not
        /// a page essential's. The model has no `tenant_id` column, so only an
        /// override can scope it.
        fn dummy_table(cx: &Cx) -> Table<Dummy> {
            Table::r#for(cx)
                .id(|d: &Dummy| d.id.to_string())
                .columns(TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                    d.name.clone()
                }))
        }

        struct UndiscoverableResource;
        impl Resource for UndiscoverableResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn requires_tenant() -> bool {
                true
            }
            fn table(cx: &Cx) -> Table<Dummy> {
                dummy_table(cx)
            }
        }

        /// The same undiscoverable model, scoped by the resource itself — the
        /// shape a row that inherits its tenant uses. `name` stands in for the
        /// relation path; the point is that the probe accepts a declared
        /// predicate.
        struct DeclaredScopeResource;
        impl Resource for DeclaredScopeResource {
            type Model = Dummy;
            fn slug() -> String {
                "declared".to_string()
            }
            fn requires_tenant() -> bool {
                true
            }
            fn tenant_scope(tenant: uuid::Uuid) -> Option<toasty::stmt::Expr<bool>> {
                Some(Dummy::fields().name().eq(tenant.to_string()))
            }
            fn table(cx: &Cx) -> Table<Dummy> {
                dummy_table(cx)
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let panel = || {
            Panel::new("admin")
                .app_context(db.clone())
                .auth(crate::Auth::disabled())
        };

        let Err(error) = panel().resource::<UndiscoverableResource>().build() else {
            panic!("a gated resource with no tenant predicate must not build");
        };
        let error = format!("{error}");
        assert!(
            error.contains("UndiscoverableResource")
                && error.contains("tenant_id")
                && error.contains("tenant_scope"),
            "the error must name the resource and both ways to scope it, got {error}"
        );

        panel()
            .resource::<DeclaredScopeResource>()
            .build()
            .expect("a declared tenant_scope scopes a gated resource");
    }
}
