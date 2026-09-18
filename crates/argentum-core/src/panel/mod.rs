//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`, and registers each
//! declared [`Resource`]'s list page at `{prefix}/{slug}` (Filament-style
//! routes — ADR-0008). See `CONTEXT.md`.
//!
//! Layout: [`Panel`] builder + route table live here; shell rendering in
//! `shell`, list + live shard support in `list`, form decoding and
//! create/edit in `forms`, delete/bulk/export in `actions`, and the
//! live-search registry + shard dispatch in `search`.

mod actions;
mod forms;
mod list;
mod search;
mod shell;

#[cfg(feature = "auth")]
pub(crate) use self::forms::parse_form_values;
pub(crate) use self::search::table_search;
pub use self::shell::{Brand, DarkMode};

use std::collections::HashMap;

use toasty::Db;
use topcoat::router::Path;
use topcoat::runtime::RouterBuilderRuntimeExt;
use topcoat::{
    Result,
    asset::{Asset, AssetConfig, RouterBuilderAssetExt},
    context::{Cx, app_context},
    cookie::RouterBuilderCookieExt,
    font::Font,
    router::{
        Body, PageFn, RouteFn, RouteFuture, Router, RouterBuilderDiscoverExt, error::redirect,
    },
};

use self::actions::{resource_bulk_delete, resource_delete, resource_export, resource_options};
use self::forms::{
    MAX_FORM_BYTES, resource_create, resource_create_post, resource_edit, resource_edit_post,
};
use self::list::resource_list;
use self::search::{SearchFn, SearchRegistry, search_handler_for};
use self::shell::ShellAssets;
use crate::resource::{NavigationItem, Resource};

/// The admin application.
///
/// ```ignore
/// Panel::new("admin")
///     .app_context(db)
///     .resource::<UserResource>()
///     .build()
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
    #[cfg(feature = "auth")]
    login_hint: Option<String>,
    #[cfg(feature = "auth")]
    auth: crate::auth::Auth,
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
            #[cfg(feature = "auth")]
            login_hint: None,
            #[cfg(feature = "auth")]
            auth: crate::auth::Auth::default(),
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
    /// first declared resource's list. Multiple calls compose; navigation
    /// order follows declaration order.
    ///
    /// Panics on duplicate slugs (GH #102): two resources over the same slug
    /// would shadow each other's routes with last-wins semantics.
    pub fn resource<R: Resource>(mut self) -> Self {
        let slug = R::slug();
        assert!(
            !self.slugs.iter().any(|s| s == &slug),
            "duplicate resource slug '{slug}': each Resource needs a distinct slug (see Resource::slug)"
        );
        self.slugs.push(slug);
        let url = format!("{}/{}", self.prefix, R::slug());
        self.pages.push(PageFn::new(
            http::Method::GET,
            route_path(&url),
            resource_list::<R>,
        ));
        // Create page — GET renders form, POST handles submission.
        let create_url = format!("{}/create", url);
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
        // Edit page — GET renders hydrated form, POST handles update.
        let edit_url = format!("{}/{{id}}/edit", url);
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
        let delete_url = format!("{}/{{id}}/delete", url);
        self.pages.push(PageFn::new(
            http::Method::POST,
            route_path(&delete_url),
            resource_delete::<R>,
        ));
        // Bulk delete — POST with `ids` form field (comma-separated).
        let bulk_delete_url = format!("{}/bulk-delete", url);
        self.pages.push(PageFn::new(
            http::Method::POST,
            route_path(&bulk_delete_url),
            resource_bulk_delete::<R>,
        ));
        // CSV export — GET reusing Resource::query + Table filters/sort (ADR-0012).
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
        // resource monomorphizes its grid loader here, keyed by list path.
        self.search_handlers
            .insert(url.clone(), search_handler_for::<R>());
        if self.root_target.is_none() {
            self.root_target = Some(url);
        }
        let nav_item = self.nav_item::<R>();
        self.nav_items.push(nav_item);
        self
    }

    /// Add a manually defined sidebar item to this Panel.
    ///
    /// Resource items should normally come from [`Self::resource`]. This hook
    /// is for pages outside the resource set, where a typed
    /// [`NavigationItem::from_href`] keeps the link and active-state check in
    /// one declaration.
    pub fn navigation(mut self, item: NavigationItem) -> Self {
        self.nav_items.push(item);
        self
    }

    /// Set branding for the shell (header + sidebar). Additive `class` stays the only Shell seam.
    pub fn brand(mut self, brand: Brand) -> Self {
        self.brand = Some(brand);
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

    /// A muted line rendered under the login form, for demo credentials or
    /// deployment hints (e.g. `"Demo: admin@example.com / password"`).
    #[cfg(feature = "auth")]
    pub fn login_hint(mut self, hint: impl Into<String>) -> Self {
        self.login_hint = Some(hint.into());
        self
    }

    /// Build the [`Router`], discovering all `#[page]` / `#[layout]` / `#[shard]`
    /// items linked into the binary, mounting the browser-runtime routes
    /// (`RouterBuilderRuntimeExt::runtime`, required by `runtime::script`),
    /// installing the `Db` and the panel navigation on the `app_context`,
    /// registering each declared resource's list page, and pointing the
    /// panel root at the first resource.
    ///
    /// Panics if no `Db` was provided via [`app_context`](Self::app_context).
    pub fn build(self) -> Router {
        assert!(
            self.shell_assets.is_none() || self.assets.is_some(),
            "Panel::build requires assets when shell_assets are configured"
        );
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
            #[cfg(feature = "auth")]
            login_hint,
            #[cfg(feature = "auth")]
            auth,
        } = self;
        let db = db.expect("Panel::build requires a Db via app_context");
        #[cfg(feature = "auth")]
        crate::auth::assert_models_registered(&db, &auth);
        let mut builder = Router::builder()
            .discover()
            .runtime()
            .cookies()
            // Form bodies (urlencoded buffered, multipart streamed) share one
            // cap (GH #90): without this layer Topcoat's 2 MiB default would
            // 413 uploads the framework otherwise accepts.
            .layer(topcoat::router::BodyLimit::max(MAX_FORM_BYTES))
            .app_context(db);
        // Auth (ADR-0013): sessions plus the resolving gate under the panel
        // and runtime prefixes, and the login/logout routes. Disabled skips
        // all three but still installs the `Auth` value for the shell.
        #[cfg(feature = "auth")]
        {
            if !auth.is_disabled() {
                builder = crate::auth::install(builder, &prefix);
                let login_path = route_path(&format!("{prefix}/login"));
                let logout_path = route_path(&format!("{prefix}/logout"));
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
        builder.build()
    }
}

impl Panel {
    /// Derive a [`NavigationItem`] for `R` using this panel's mount prefix.
    ///
    /// This is the panel-aware counterpart to `NavigationItem::from_resource`.
    /// The URL respects `self.prefix()` so `Panel::new("backoffice")` yields
    /// `"/backoffice/{slug}"` instead of hard-coded `"/admin"`.
    pub(crate) fn nav_item<R: Resource>(&self) -> NavigationItem {
        NavigationItem::from_resource_with_prefix::<R>(&self.prefix)
    }
}

/// Parse a panel route path, panicking on malformed input — the paths are
/// built from the panel prefix and the resource slug, both validated earlier.
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

/// The list URL for a resource: `{panel prefix}/{slug}`.
///
/// The prefix comes from the [`PanelPrefix`] app context installed by
/// [`Panel::build`] — the panel's own declaration, not the request path. The
/// old `list_url_for_current` sniffed the current path (stripping
/// `/create`, `/{id}/edit`, `/{id}/delete`, … suffixes), which only worked
/// because every handler happened to sit under the list route and hardcoded
/// `/admin` as its fallback (GH #75 item 6). When no prefix is installed
/// (bare `CxTestBuilder` tests), fall back to the request path's first
/// segment, then `/admin`.
pub(crate) fn list_url(cx: &Cx, slug: &str) -> String {
    let prefix = topcoat::context::try_app_context::<PanelPrefix>(cx)
        .map(|p| p.0.clone())
        .unwrap_or_else(|| {
            let path = topcoat::router::request::uri(cx).path().to_string();
            path.split('/')
                .nth(1)
                .filter(|s| !s.is_empty())
                .map(|s| format!("/{s}"))
                .unwrap_or_else(|| "/admin".to_string())
        });
    format!("{prefix}/{slug}")
}

/// The panel root: a temporary redirect to the first declared resource's
/// list, so the mount point is never a dead URL (custom pages remain future
/// work, see README §10). Filament registers its home page here.
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
    use super::*;
    use toasty::Db;

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

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
        }

        let db = Db::builder().connect("sqlite::memory:").await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<DummyResource>()
            .auth(crate::Auth::disabled())
            .build();

        // The list page denies by default (default-deny policy → 403). A
        // POST through the runtime's page-rerun route rewrites into a GET
        // for the page, so it reaches the handler and reports 403; without
        // `.runtime()` on the builder there would be no such route (404).
        // (Topcoat #391: `runtime::script` requires these routes.)
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("/_topcoat/runtime/pages/admin/dummies")
            .header("content-type", "application/json")
            .body(Body::from("{}".to_owned()))
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
    }

    /// The panel root answers the gate before reading `RootRedirect`
    /// (GH #146 defense in depth): a mis-mounted gate must not leak the
    /// first resource's slug via the redirect target.
    #[tokio::test]
    async fn panel_root_redirect_rechecks_auth_before_the_root_target() {
        use topcoat::context::CxTestBuilder;
        use topcoat::router::response::IntoResponse;

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

    #[test]
    #[should_panic(expected = "Panel::build requires a Db")]
    fn panel_build_panics_without_db() {
        let _router = Panel::new("admin").build();
    }

    #[test]
    fn panel_navigation_item_respects_prefix() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
        }

        let panel = Panel::new("backoffice");
        let item = panel.nav_item::<DummyResource>();
        // Label: pluralized model name ("Dummy" → "Dummies"); URL: prefix +
        // resource slug ("DummyResource" → "dummies").
        assert_eq!(item.label, "Dummies");
        assert_eq!(item.url, "/backoffice/dummies");

        let default = Panel::new("admin").nav_item::<DummyResource>();
        assert_eq!(default.url, "/admin/dummies");
        // The prefix-aware constructor normalises slashes
        let via_prefix = crate::resource::NavigationItem::from_resource_with_prefix::<DummyResource>(
            "/backoffice/",
        );
        assert_eq!(via_prefix.url, "/backoffice/dummies");
    }

    #[test]
    fn panel_navigation_items_are_distinct_for_multiple_resources() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
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
        assert_eq!(users.url, "/admin/users");
        assert_eq!(categories.url, "/admin/categories");
        assert_ne!(users.url, categories.url);
    }

    #[test]
    #[should_panic(expected = "duplicate resource slug")]
    fn panel_rejects_duplicate_resource_slugs() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
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

        let _ = Panel::new("admin")
            .resource::<FirstResource>()
            .resource::<SecondResource>();
    }
}
