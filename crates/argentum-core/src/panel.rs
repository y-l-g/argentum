//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`, and registers each
//! declared [`Resource`]'s list page at `{prefix}/{slug}` (Filament-style
//! routes — ADR-0008). See `CONTEXT.md`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::header::COOKIE;
use toasty::Db;
use topcoat::runtime::{Event, Signal, shard, signal};
use topcoat::view::internal::ThenView;
use topcoat::{
    Result,
    asset::{Asset, AssetConfig, RouterBuilderAssetExt},
    context::{Cx, app_context, try_request_context},
    cookie::RouterBuilderCookieExt,
    font::Font,
    router::{
        Body, PageFn, RouteFn, RouteFuture, Router, RouterBuilderDiscoverExt, Slot,
        error::{forbidden, redirect, see_other},
        request::{Bytes, FromRequest},
    },
    view::{BoxView, Child, HoistView, View, ViewExt, attributes, suspense, view},
};

use crate::db::db;
use crate::notification::{
    LiveToast, Notification, live_toast, live_toaster, set_notification, take_notification,
};
use crate::resource::{NavigationItem, Resource, Table, TablePage, TableState};
use topcoat::router::Path;
use topcoat::runtime::RouterBuilderRuntimeExt;

/// The admin application.
///
/// ```ignore
/// Panel::new("admin")
///     .app_context(db)
///     .resource::<UserResource>()
///     .build()
/// ```
/// Branding for the admin shell (panel header + sidebar header).
#[derive(Debug, Clone)]
pub struct Brand {
    /// Display name (e.g. `"Acme"`).
    pub name: String,
    /// Optional logo URL (e.g. `"/logo.svg"`). Rendered as an `<img>` when present.
    pub logo: Option<String>,
}

impl Brand {
    /// Create a brand with the given name (GH #102: surrounding whitespace is
    /// trimmed so `" Acme "` cannot break the `flex h-16` header).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into().trim().to_string(),
            logo: None,
        }
    }

    /// Attach a logo URL (GH #102: blank values are ignored so an empty
    /// `logo("")` falls back to the name-only render instead of a
    /// broken-image icon).
    pub fn logo(mut self, logo: impl Into<String>) -> Self {
        let logo = logo.into().trim().to_string();
        if !logo.is_empty() {
            self.logo = Some(logo);
        }
        self
    }
}

/// Whether the shell starts in dark mode. Persisted via `theme.js` (`localStorage` + `theme` cookie).
///
/// Precedence (GH #102): this build-time default only sets the initial
/// `<html class>` — the blocking `theme_init_script` + `theme.js` correct it
/// pre-paint from `localStorage` first, then the `theme` cookie. The server
/// never reads the cookie per request; a user toggle wins over this default
/// on every later visit.
#[derive(Debug, Clone, Copy)]
pub struct DarkMode(pub bool);

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

/// The application-owned assets used by [`Panel::layout_shell`].
///
/// Tailwind's generated stylesheet is necessarily a call-site asset because
/// every application scans a different source tree. The Panel therefore takes
/// the generated stylesheet and the application's chosen font as values while
/// still owning the document markup that links them.
#[derive(Debug, Clone, Copy)]
struct ShellAssets {
    stylesheet: Asset,
    font: Font,
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
        // Filament's panel root is a Dashboard; until dashboards exist,
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

    /// Derive a [`NavigationItem`] for `R` using this panel's mount prefix.
    ///
    /// This is the panel-aware counterpart to `NavigationItem::from_resource`.
    /// The URL respects `self.prefix()` so `Panel::new("backoffice")` yields
    /// `"/backoffice/{slug}"` instead of hard-coded `"/admin"`.
    pub fn nav_item<R: Resource>(&self) -> NavigationItem {
        NavigationItem::from_resource_with_prefix::<R>(&self.prefix)
    }

    async fn theme_toggle(cx: &Cx) -> Result<BoxView<'_>> {
        use argentum_ui::{ButtonSize, ButtonVariant, button};

        Ok(view! {
            cx =>
            button(
                variant: ButtonVariant::Ghost,
                size: ButtonSize::Icon,
                attrs: attributes! { aria-label="Toggle dark mode" data-theme-toggle="" },
                <span aria-hidden="true">"◐"</span>
            )
        }
        .boxed())
    }

    pub(crate) async fn render_brand(cx: &Cx) -> Result<BoxView<'_>> {
        use topcoat::context::try_app_context;
        let (name, logo) = if let Some(brand) = try_app_context::<Brand>(cx) {
            (brand.name.clone(), brand.logo.clone())
        } else {
            ("Argentum".to_string(), None)
        };
        if let Some(logo_url) = logo {
            let alt = name.clone();
            Ok(view! {
                cx =>
                <div class="flex items-center gap-2 font-semibold text-foreground">
                    <img
                        src=(logo_url)
                        alt=(alt)
                        width="24"
                        height="24"
                        class="h-6 w-6 rounded"
                    >
                    (name)
                </div>
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                <div class="flex items-center gap-2 font-semibold text-foreground">
                    (name)
                </div>
            }
            .boxed())
        }
    }

    async fn sidebar_navigation<'a>(
        cx: &'a Cx,
        nav_items: &[NavigationItem],
        current_path: &str,
        mobile_open: Signal<bool>,
    ) -> Result<BoxView<'a>> {
        use argentum_ui::{
            sidebar_group, sidebar_group_content, sidebar_group_label, sidebar_menu,
            sidebar_menu_button, sidebar_menu_item,
        };
        let mut nav_items = nav_items.to_vec();
        // Stable order (GH #102): explicit `order` first, declaration order
        // breaking ties — custom items interleave via `.sorted()`.
        nav_items.sort_by_key(|item| item.order);
        let current_path = current_path.to_string();

        Ok(view! {
            cx =>
            sidebar_group(
                sidebar_group_label("Navigation")
                sidebar_group_content(
                    sidebar_menu(
                        for item in &nav_items {
                            // Prefer typed Href when available, else fallback to path string.
                            let is_active = if item.href_check.is_some() {
                                item.is_current(cx)
                            } else {
                                item.is_current_path(&current_path)
                            };
                            sidebar_menu_item(
                                sidebar_menu_button(
                                    active: is_active,
                                    href: Some(item.url.as_str()),
                                    tooltip: Some(item.label.as_str()),
                                    attrs: attributes! {
                                        // Tapping a link in the mobile sheet closes
                                        // it; on desktop the navigation is the effect.
                                        @click=$(|_e: Event| mobile_open.set(false))
                                    },
                                    <span>(item.label.clone())</span>
                                )
                            )
                        }
                    )
                )
            )
        }
        .boxed())
    }

    /// Whether the persisted `sidebar_state` cookie asks for an expanded
    /// desktop sidebar (default: expanded).
    ///
    /// Parsed from the raw `Cookie` header on purpose: `topcoat::cookie::cookies`
    /// panics when the cookie router layer is absent (tests, minimal routers),
    /// and the shell must render everywhere. The value only seeds the runtime
    /// signal's initial `data-state`; after hydration the browser owns the
    /// state, and `assets/sidebar.js` mirrors changes back to the cookie.
    fn sidebar_starts_open(cx: &Cx) -> bool {
        !try_request_context::<http::request::Parts>(cx)
            .and_then(|parts| parts.headers.get(COOKIE))
            .and_then(|value| value.to_str().ok())
            .is_some_and(|cookie| {
                cookie
                    .split(';')
                    .any(|part| part.trim().strip_prefix("sidebar_state=") == Some("collapsed"))
            })
    }

    /// Render the Filament-grade Shell that frames every admin page.
    ///
    /// Composes Topcoat's upstream `sidebar` primitives (ADR-0007): the
    /// desktop panel and the mobile sheet share one navigation rendering, and
    /// the open state lives in runtime signals — `open` seeds from the
    /// `sidebar_state` cookie for the first paint, the triggers carry
    /// `@click` handlers, and `assets/sidebar.js` persists changes back to
    /// the cookie. Includes dark-mode toggle (Ghost button, persists via
    /// cookie/session) and the toast stack (shadcn/Sonner surface, fixed
    /// bottom-right). Additive `class` is allowed on the outer container only
    /// (narrow seam).
    pub async fn render_shell<'a>(
        cx: &'a Cx,
        nav_items: &[NavigationItem],
        current_path: &str,
        slot: Child<'a>,
        extra_class: Option<String>,
    ) -> Result<BoxView<'a>> {
        // A hoisting body: the sidebar signals are declared while this view
        // resolves, and a page re-run resumes them from the client. The owned
        // copies pin the caller's navigation to the lazy body's lifetime.
        let nav_items = nav_items.to_vec();
        let current_path = current_path.to_string();
        Ok(Box::pin(HoistView::new(ThenView::new(async move {
            Self::render_shell_body(cx, &nav_items, &current_path, slot, extra_class).await
        }))))
    }

    async fn render_shell_body<'a>(
        cx: &'a Cx,
        nav_items: &[NavigationItem],
        current_path: &str,
        slot: Child<'a>,
        extra_class: Option<String>,
    ) -> Result<BoxView<'a>> {
        use argentum_ui::{
            SeparatorOrientation, SidebarCollapsible, separator, sidebar, sidebar_content,
            sidebar_footer, sidebar_header, sidebar_inset, sidebar_provider, sidebar_trigger,
        };

        let sidebar_open = signal(cx, || Self::sidebar_starts_open(cx));
        let mobile_open = signal(cx, || false);
        let outer_class = extra_class.clone().unwrap_or_default();
        let header_title = topcoat::context::try_app_context::<Brand>(cx)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Admin".to_string());
        // One navigation tree: the upstream sidebar renders its children once
        // and shares them between the desktop panel and the mobile sheet.
        let navigation =
            Self::sidebar_navigation(cx, nav_items, current_path, mobile_open.clone()).await?;
        let sidebar_brand = Self::render_brand(cx).await?;
        let sidebar_theme_toggle = Self::theme_toggle(cx).await?;
        let header_theme_toggle = Self::theme_toggle(cx).await?;
        // Signed-in identity + logout control, present only with a session
        // (ADR-0013). `ensure_token` runs before any streaming starts so the
        // logout form always carries a matching CSRF pair.
        #[cfg(feature = "auth")]
        let account_view: BoxView<'_> = match crate::auth::current_user(cx) {
            Some(user) => {
                let csrf = crate::csrf::ensure_token(cx);
                let logout = crate::auth::logout_url(cx);
                view! {
                    cx =>
                    <div class="flex items-center gap-2">
                        <span class="text-sm text-muted-foreground">
                            (user.display_name)
                        </span>
                        <form method="post" action=(logout)>
                            <input
                                type="hidden"
                                name=(crate::csrf::FIELD_NAME)
                                value=(csrf)
                            >
                            <button
                                type="submit"
                                class="text-sm text-muted-foreground underline"
                            >
                                "Sign out"
                            </button>
                        </form>
                    </div>
                }
                .boxed()
            }
            None => view! { cx => <span></span> }.boxed(),
        };
        #[cfg(not(feature = "auth"))]
        let account_view: BoxView<'_> = view! { cx => <span></span> }.boxed();
        let notification_view: BoxView<'_> = if let Some(notification) = take_notification(cx) {
            crate::notification::render_notification(cx, notification, Default::default()).await?
        } else {
            view! { cx => <span></span> }.boxed()
        };
        // The page owns the live-toast signals; resolve the same handles here
        // (same helper, same request identity) and hand them to the shard
        // (GH #154 §3).
        let LiveToast {
            status: toast_status,
            title: toast_title,
            description: toast_description,
            serial: toast_serial,
        } = live_toast(cx);

        Ok(view! {
            cx =>
            sidebar_provider(
                attrs: attributes! { class=(outer_class) },
                // `sidebar_rail` is intentionally not rendered: the header
                // `sidebar_trigger` is the explicit toggle, and the rail's
                // edge hit-area reads as stray chrome as a primary toggle.
                // Keep the component available (`argentum_ui::sidebar_rail`)
                // for opt-in `variant=inset`/`floating` layouts.
                sidebar(
                    open: $(sidebar_open.get()),
                    mobile_open: $(mobile_open.get()),
                    collapsible: SidebarCollapsible::Offcanvas,
                    sheet_attrs: attributes! {
                        id="mobile-sidebar-sheet"
                        aria-label="Navigation"
                        @keydown=$(|e: Event| {
                            if e.key == "Escape" {
                                mobile_open.set(false);
                            }
                        })
                        @click=$(|e: Event| {
                            if e.target.id == "mobile-sidebar-sheet" {
                                mobile_open.set(false);
                            }
                        })
                    },
                    sidebar_header((sidebar_brand))
                    sidebar_content((navigation))
                    sidebar_footer((sidebar_theme_toggle))
                )
                sidebar_inset(
                    sidebar_header(
                        // The desktop trigger collapses the rail; below md the
                        // mobile trigger opens the sheet instead (shadcn
                        // SidebarTrigger pair, upstream `examples/ui`).
                        sidebar_trigger(
                            open: $(sidebar_open.get()),
                            attrs: attributes! {
                                class="max-md:hidden"
                                aria-controls="mobile-sidebar-sheet"
                                @click=$(|_e: Event| sidebar_open.toggle())
                            }
                        )
                        sidebar_trigger(
                            open: $(mobile_open.get()),
                            attrs: attributes! {
                                class="md:hidden"
                                aria-controls="mobile-sidebar-sheet"
                                @click=$(|_e: Event| mobile_open.toggle())
                            }
                        )
                        separator(orientation: SeparatorOrientation::Vertical)
                        <div class="font-semibold text-foreground">(header_title)</div>
                        <div class="ml-auto flex items-center gap-2">
                            (account_view)
                            (header_theme_toggle)
                        </div>
                    )
                    <main class="flex-1 mx-auto max-w-7xl w-full p-6">(slot)</main>
                )
                // Toast stack — the shadcn/Sonner surface, fixed bottom-right
                // and a polite live region so streamed swaps are announced
                // (GH #98, GH #151). `live_toaster` is the page-owned
                // in-place transport (GH #154 §3); the flash cookie's toast
                // rides beside it.
                argentum_ui::toaster(
                    (notification_view)
                    live_toaster(
                        status: $(toast_status),
                        title: $(toast_title),
                        description: $(toast_description),
                        serial: $(toast_serial)
                    )
                )
            ) // Scripts are owned by the document (layout_shell).
        }
        .boxed())
    }

    /// Convenience wrapper for `#[layout]` handlers: takes the layout's
    /// `slot: Slot<'_>` and renders the complete HTML document around the
    /// shell.
    ///
    /// The stylesheet and font are supplied to the Panel builder with
    /// [`Self::shell_assets`]. A Panel without those values remains renderable
    /// for tests and custom document owners, but does not pretend that a CSS
    /// bundle exists. Errors from the page slot propagate unchanged when the
    /// document view is resolved.
    pub async fn layout_shell<'a>(cx: &'a Cx, slot: Slot<'a>) -> Result<impl View + 'a> {
        use topcoat::context::try_app_context;
        use topcoat::router::request::uri;
        let current = uri(cx).path().to_string();
        // Prefer declarative nav_items from Panel::resource, fallback to Dashboard.
        let nav_items = try_app_context::<Vec<NavigationItem>>(cx)
            .cloned()
            .unwrap_or_else(|| {
                vec![NavigationItem {
                    label: "Dashboard".to_string(),
                    url: "/admin".to_string(),
                    href_check: None,
                    order: 0,
                }]
            });
        let shell = Self::render_shell(cx, &nav_items, &current, slot, None).await?;
        let brand_title = try_app_context::<Brand>(cx)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Admin".to_string());
        Self::render_document(cx, brand_title, shell).await
    }

    /// The complete HTML document around a rendered body: assets, dark-mode
    /// class, and title. [`Self::layout_shell`] frames the panel shell with
    /// it; the standalone login page (ADR-0013) uses the same document so
    /// brand and dark mode carry over.
    pub(crate) async fn render_document<'a>(
        cx: &'a Cx,
        title: String,
        body: BoxView<'a>,
    ) -> Result<BoxView<'a>> {
        use topcoat::context::try_app_context;
        let head: BoxView<'_> = match try_app_context::<ShellAssets>(cx).copied() {
            Some(ShellAssets { stylesheet, font }) => view! {
                cx =>
                topcoat::dev::script()
                argentum_ui::theme_init_script()
                topcoat::runtime::script()
                topcoat::font::link(font: font)
                <link rel="stylesheet" href=(stylesheet)>
                <script src=(argentum_ui::SIDEBAR_JS)></script>
                <script src=(argentum_ui::THEME_JS)></script>
                <script src=(argentum_ui::DIALOG_JS)></script>
                <script src=(argentum_ui::CODE_BLOCK_JS)></script>
                <script src=(argentum_ui::BULK_JS)></script>
                <script src=(argentum_ui::FILTERS_JS)></script>
                <script src=(argentum_ui::SELECTS_JS)></script>
                <script src=(argentum_ui::NOTIFICATION_JS)></script>
            }
            .boxed(),
            None => view! {
                cx =>
                topcoat::dev::script()
                argentum_ui::theme_init_script()
            }
            .boxed(),
        };
        let html_class =
            try_app_context::<DarkMode>(cx).and_then(|dm| if dm.0 { Some("dark") } else { None });
        Ok(view! {
            cx =>
            <!DOCTYPE html>
            <html class=(html_class)>
                <head>
                    <title>(title)</title>
                    (head)
                </head>
                <body>(body)</body>
            </html>
        }
        .boxed())
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
fn enforce_auth(cx: &Cx) -> Result<(), topcoat::Error> {
    if crate::auth::enforced(cx) {
        crate::auth::require_authenticated(cx)?;
    }
    Ok(())
}

/// Auth compiled out: the gate does not exist either, so nothing to enforce.
#[cfg(not(feature = "auth"))]
fn enforce_auth(_cx: &Cx) -> Result<(), topcoat::Error> {
    Ok(())
}

/// Enforce tenancy gating for resources that require it (GH #87).
///
/// Wired into every resource handler; a no-op unless the resource overrides
/// `Resource::requires_tenant`. Fails closed (403) when no tenant is present
/// instead of serving unscoped rows.
fn enforce_tenant<R: Resource>(cx: &Cx) -> Result<(), topcoat::Error> {
    if R::requires_tenant() {
        crate::tenancy::require_tenant(cx)?;
    }
    Ok(())
}

/// The list page every declared [`Resource`] gets at `{prefix}/{slug}`.
///
/// One generic handler drives all resources: resolve the [`TableState`] from
/// the URL, scope through `Resource::query` (the tenancy seam, ADR-0002),
/// apply the table's search/sort/pagination declarations, render through
/// `Resource::table`. The page title is the resource's navigation label.
///
/// The page streams: shell and header go out with the first content, while the
/// row grid (toolbar/filter/bulk/pager included) loads inside a `suspense`
/// region that swaps in the skeleton → table without any client-side fetching
/// (GH #98: the skeleton is thead + placeholders only, so chrome pops in with
/// the swap by design).
/// A monomorphized live-search grid loader, one per declared resource.
///
/// `#[shard]` inventory only discovers concrete fns (GH #104), so the single
/// concrete [`table_search`] shard dispatches through this registry instead
/// of going generic. Built by [`Panel::resource`], keyed by list path.
type SearchFn = Arc<
    dyn for<'a> Fn(
            &'a Cx,
            TableState,
            String,
            crate::resource::TableSignals,
        ) -> Pin<Box<dyn Future<Output = Result<BoxView<'a>>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Live-search handlers installed on the app context by [`Panel::build`].
#[derive(Clone, Default)]
pub struct SearchRegistry(pub HashMap<String, SearchFn>);

/// Monomorphize `R`'s grid loader into a [`SearchFn`]: tenancy + policy gate,
/// then the same load + render the streamed list uses.
fn search_handler_for<R: Resource>() -> SearchFn {
    Arc::new(
        |cx: &Cx,
         state: TableState,
         path: String,
         signals: crate::resource::TableSignals|
         -> Pin<Box<dyn Future<Output = Result<BoxView<'_>>> + Send + '_>> {
            Box::pin(async move {
                enforce_auth(cx)?;
                enforce_tenant::<R>(cx)?;
                if !R::can_view_any(cx) {
                    return Err(forbidden().into());
                }
                // The swapped region is everything EXCEPT the search toolbar:
                // the live host (input + this invocation) already owns that
                // slot on the page, and re-rendering it per keystroke would
                // nest invocations and duplicate inputs. Forcing the GET form
                // off also keeps signals out of the swap payload.
                let mut table = R::table(cx).without_skeleton().search(false);
                table = if R::deletable() {
                    table
                        .with_delete(list_url(cx, &R::slug()))
                        .with_bulk_delete(true)
                } else {
                    table
                };
                let page = load_table_page::<R>(cx, &table, &state).await?;
                table
                    .render_live_with_state(cx, page, &state, &path, signals)
                    .await
            })
        },
    )
}

/// Resolve the registered live-search handler for `path`, answering the gate
/// first (GH #146 defense in depth): the registry lookup runs only for an
/// authenticated request, so an unknown `path` cannot be distinguished from a
/// registered one by an unauthenticated probe (404-vs-401 oracle).
fn search_entry(cx: &Cx, path: &str) -> Result<SearchFn> {
    enforce_auth(cx)?;
    topcoat::context::try_app_context::<SearchRegistry>(cx)
        .and_then(|reg| reg.0.get(path).cloned())
        .ok_or_else(|| topcoat::router::error::not_found().into())
}

/// Live table interactions (GH #104, GH #151): re-renders one resource's grid
/// as its signals change, morphing in place per Topcoat #392 (focus, scroll,
/// and typing survive; rows carry stable `id`s from #104 prep).
///
/// The shard owns no state: the page creates the signals ([`TableSignals`]),
/// renders the toolbar against them, and passes their handles here. Search,
/// sort, filters, and pagination all write those signals, so one dependency
/// graph re-renders the grid — no navigation, no scroll jump. The swapped
/// region is the grid without the search toolbar (the live host owns that
/// slot, so swaps never nest invocations or duplicate inputs).
///
/// Every arg is untrusted shard input: `path` must name a registered list
/// (allow-list, never a raw route), and every signal value is clamped or
/// re-parsed through [`TableState::from_live_args`] like the GET path.
/// Authorization mirrors the list page (`requires_tenant` + `can_view_any`,
/// row scoping via `Resource::query`); shard POSTs carry no CSRF token, and
/// none is needed for this read-only rerun. The GET toolbar stays as the
/// no-JS fallback.
///
/// The module exists only to carry `allow(too_many_arguments)`: the shard's
/// arity is its dependency list (one signal per interaction), and the macro
/// expands the handler past the lint's default.
#[allow(clippy::too_many_arguments)]
mod shard_body {
    use super::*;

    #[shard]
    pub(crate) async fn table_search(
        cx: &Cx,
        path: String,
        q: topcoat::runtime::Signal<String>,
        filters: topcoat::runtime::Signal<String>,
        sort: topcoat::runtime::Signal<String>,
        dir: topcoat::runtime::Signal<String>,
        after: topcoat::runtime::Signal<String>,
        before: topcoat::runtime::Signal<String>,
        group_by: String,
    ) -> Result<impl View> {
        let entry = search_entry(cx, &path)?;
        let signals = crate::resource::TableSignals {
            q,
            filters,
            sort,
            dir,
            after,
            before,
        };
        // One shared bound (GH #148): the GET `?q=` path and the shard clamp
        // through the same helper, so a term too long for the URL is too long
        // here. Cursors are honored as sent: search/sort/filter handlers clear
        // them when the result set changes, so a live cursor always belongs to
        // the current query.
        let q = crate::resource::clamp_query_term(&signals.q.get());
        let mut state = TableState::from_live_args(
            &q,
            &signals.filters.get(),
            &signals.sort.get(),
            &signals.dir.get(),
            &group_by,
        );
        let after = signals.after.get();
        if !after.trim().is_empty() {
            state.after = Some(after.trim().to_string());
        }
        let before = signals.before.get();
        if !before.trim().is_empty() {
            state.before = Some(before.trim().to_string());
        }
        entry(cx, state, path, signals).await
    }
}
pub(crate) use shard_body::table_search;

/// Retry link for a failed streamed grid load (GH #110).
///
/// A malformed `?after=`/`?before=` cursor is the failure itself: retrying the
/// identical URL loops forever, so drop pagination from the link and keep the
/// rest of the state (search/sort/filters/grouping). Every other failure keeps
/// pagination too (GH #98) so a transient blip retries the same evidence.
fn retry_url_for_error(state: &TableState, error: &topcoat::Error, path: &str) -> String {
    if error
        .downcast_ref::<crate::cursor::CursorDecodeError>()
        .is_some()
    {
        state.retry_url_without_cursor(path)
    } else {
        state.retry_url(path)
    }
}

fn resource_list<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_view_any(cx) {
            return Err(forbidden().into());
        }
        // Ensure the CSRF cookie before streaming starts (GH #99): streamed
        // children can only read it via current_token.
        crate::csrf::ensure_token(cx);
        let state = TableState::from_cx(cx);
        let mut table = R::table(cx);
        // Wire delete/bulk-delete action base from the Panel prefix — the
        // delete form posts to `{list url}/{id}/delete` and the bulk bar to
        // `{list url}/bulk-delete`, both derived from the panel declaration
        // (not the request path) so the URLs are right wherever the table
        // renders. Read-only resources opt out via `Resource::deletable`
        // (GH #96) instead of rendering buttons that always 403.
        table = if R::deletable() {
            table
                .with_delete(list_url(cx, &R::slug()))
                .with_bulk_delete(true)
        } else {
            table
        };
        let title = R::navigation_label();
        let list_path = list_url(cx, &R::slug());
        if table.is_live_search() {
            return Ok(resource_list_live::<R>(cx, table, state, title, list_path));
        }

        // First content: the skeleton grid (same markup the eager
        // `defer(true)` path renders), while the rows load below. The load
        // catches its own errors: post-stream the status line is fixed,
        // so a failed load must render the branded ErrorState
        // inside the region instead of truncating the body. Pre-stream
        // failures (e.g. the skeleton itself) still propagate and map onto
        // the response status. (For children that partially stream before
        // failing, topcoat's `error_boundary` is the replace-in-place seam.)
        let skeleton = table.render_skeleton(cx).await?;
        // The swap payload must be rows even when the declared table sets
        // `.defer(true)` (GH #98 trap: render() would return a second skeleton).
        let table = table.without_skeleton();
        let lazy_rows = ThenView::new(async move {
            let grid = async {
                let page = load_table_page::<R>(cx, &table, &state).await?;
                table.render(cx, page).await
            };
            match grid.await {
                Ok(view) => Ok(view),
                Err(error) => {
                    tracing::error!(resource = R::slug(), error = %error, "table load failed");
                    let retry = retry_url_for_error(&state, &error, &list_url(cx, &R::slug()));
                    let action = view! { cx => <a href=(retry)>"Retry"</a> }.boxed();
                    Ok(view! {
                        cx =>
                        argentum_ui::error_state(
                            title: format!("Couldn't load {}", R::navigation_label()),
                            detail: "Something went wrong while loading the records.",
                            action: Some(action.into()),
                            attrs: attributes! { role="alert" }
                        )
                    }
                    .boxed())
                }
            }
        });

        Ok(view! {
            cx =>
            argentum_ui::page(
                argentum_ui::page_header(argentum_ui::page_title((title.clone())))
                argentum_ui::page_content(
                    <div class="flex flex-col gap-4">
                        suspense(fallback: skeleton, (lazy_rows.boxed()))
                    </div>
                )
            )
        }
        .boxed())
    })))
}

/// Live list page for `Table::live_search` tables (GH #104, GH #151): the
/// page owns the interaction signals (`q`, `filters`, `sort`, `dir`,
/// `after`, `before`) and renders the search toolbar eagerly above the
/// streamed region while the `table_search` shard invocation fills the grid
/// below — one grid per response, so rows can never duplicate. Every
/// interaction writes a signal, so search, sort, filters, and pagination
/// re-render only the invocation output, morphing in place with focus and
/// scroll surviving.
fn resource_list_live<R: Resource>(
    cx: &Cx,
    table: Table<R::Model>,
    state: TableState,
    title: String,
    list_path: String,
) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        use crate::resource::TableSignals;
        use topcoat::runtime::signal;

        let signals = TableSignals {
            q: signal(cx, || state.search.clone().unwrap_or_default()),
            filters: signal(cx, || state.filters_param().unwrap_or_default()),
            sort: signal(cx, || {
                state
                    .sort
                    .as_ref()
                    .map(|s| s.column.clone())
                    .unwrap_or_default()
            }),
            dir: signal(cx, || {
                state
                    .sort
                    .as_ref()
                    .map(|s| if s.descending { "desc" } else { "asc" })
                    .unwrap_or("asc")
                    .to_string()
            }),
            after: signal(cx, || state.after.clone().unwrap_or_default()),
            before: signal(cx, || state.before.clone().unwrap_or_default()),
        };
        let host = if table.search_enabled() {
            Some(
                table
                    .render_live_search_bar(cx, &state, &list_path, &signals)
                    .await?,
            )
        } else {
            None
        };
        let skeleton = table.render_skeleton(cx).await?;
        // The delete confirmation dialog is not part of the swapped grid
        // region: a keystroke starts a new result set and must never carry
        // (or re-open) a dialog, so the live page renders it eagerly once
        // (GH #151).
        let delete_dialog = table.render_delete_dialog(cx, &state, &list_path).await?;
        // The swap payload must be rows even when the declared table sets
        // `.defer(true)` (GH #98 trap).
        let table = table.without_skeleton();
        let lazy_rows = ThenView::new(async move {
            let grid = table
                .render_live_invocation(cx, &state, &list_path, signals)
                .await;
            match grid {
                Ok(view) => Ok(view),
                Err(error) => {
                    tracing::error!(resource = R::slug(), error = %error, "table load failed");
                    let retry = retry_url_for_error(&state, &error, &list_path);
                    let action = view! { cx => <a href=(retry)>"Retry"</a> }.boxed();
                    Ok(view! {
                        cx =>
                        argentum_ui::error_state(
                            title: format!("Couldn't load {}", R::navigation_label()),
                            detail: "Something went wrong while loading the records.",
                            action: Some(action.into()),
                            attrs: attributes! { role="alert" }
                        )
                    }
                    .boxed())
                }
            }
        });

        Ok(view! {
            cx =>
            argentum_ui::page(
                argentum_ui::page_header(argentum_ui::page_title((title.clone())))
                argentum_ui::page_content(
                    <div class="flex flex-col gap-4">
                        if let Some(host) = host {
                            (host)
                        }
                        suspense(fallback: skeleton, (lazy_rows.boxed()))
                        if let Some(dialog) = delete_dialog {
                            (dialog)
                        }
                    </div>
                )
            )
        })
    })))
}

/// Resolve the declared table (search / filters / sort / pagination) against
/// `Resource::query` and execute it — the data-loading half of
/// [`resource_list`], kept separate so the page shell can stream before it.
async fn load_table_page<R: Resource>(
    cx: &Cx,
    table: &Table<R::Model>,
    state: &TableState,
) -> Result<TablePage<R::Model>> {
    table.load(cx, R::query(cx), state).await
}

/// Helper: parse form bodies into a map — `application/x-www-form-urlencoded`
/// buffered, plus `multipart/form-data` streamed when a `FileUpload` is present
/// (GH #73).
///
/// Decoding for urlencoded is delegated to `form_urlencoded` (already in the
/// tree via topcoat): it splits pairs, decodes `+` as space, assembles
/// multi-byte UTF-8 from `%XX` sequences (`%C3%A9` → `é`, not `Ã©`), and keeps
/// encoded separators (`%26` → `&`) intact — the hand-rolled `percent_decode`
/// it replaced pushed each decoded byte through `byte as char`, corrupting
/// every non-ASCII value (GH #75 item 6). Invalid UTF-8 degrades per-value
/// (lossy) instead of discarding the whole form.
///
/// Multipart (file) parts stream through Topcoat's multer-based extractor
/// (GH #90): file bytes are drained in chunks and discarded — v1 stores the
/// sanitized filename as the `String` value, never the bytes (see
/// `FileUpload` storage contract) — so a 2 GB "upload" never materializes.
/// Text parts store their content. Unknown content types fall back to
/// urlencoded so existing tests/clients keep working.
pub(crate) async fn parse_form_values(
    cx: &Cx,
    body: Body,
) -> Result<HashMap<String, String>, topcoat::Error> {
    let content_type =
        topcoat::context::try_request_context::<http::request::Parts>(cx).and_then(|parts| {
            parts
                .headers
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
        });
    if content_type
        .as_deref()
        .is_some_and(is_multipart_content_type)
    {
        return parse_multipart_values(cx, body).await;
    }
    let bytes = Bytes::from_request(cx, body)
        .await
        .map_err(|_| topcoat::router::error::bad_request("cannot read form body"))?;
    form_values_from_request_parts(content_type.as_deref(), bytes.as_ref())
}

/// Whether a content type is `multipart/form-data` (parameters ignored).
fn is_multipart_content_type(ct: &str) -> bool {
    ct.split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
}

/// Streamed multipart half of [`parse_form_values`] (GH #90).
///
/// Fields stream one at a time with constant memory: text fields buffer
/// (bounded by the request body limit), file fields drain-and-discard while
/// only the sanitized filename is kept. Duplicate part names are last-wins;
/// nameless parts are skipped. A missing boundary is a 400, an over-limit
/// body a 413 — both classified by the extractor, never silent fallbacks.
/// Every byte the stream carries is also counted against [`MAX_FORM_BYTES`]
/// (GH #149): file drains and skipped parts go through the counter chunk by
/// chunk, text fields join it after their (extractor-bounded) read, so a
/// large upload cannot be drained chunk-by-chunk holding the handler even if
/// the extractor's limit stops wrapping the stream.
async fn parse_multipart_values(
    cx: &Cx,
    body: Body,
) -> Result<HashMap<String, String>, topcoat::Error> {
    use topcoat::router::content::multipart::Multipart;
    use topcoat::router::request::FromRequest;

    let mut out = HashMap::new();
    let mut bytes_seen = 0usize;
    let mut multipart = Multipart::from_request(cx, body).await?;
    while let Some(mut field) = multipart.next_field().await? {
        let Some(name) = field.name().map(str::to_string) else {
            // Nameless parts carry bytes too: drain them through the counter
            // so the accounting covers the whole request stream (GH #149).
            drain_bounded(&mut field, &mut bytes_seen).await?;
            continue;
        };
        if name.is_empty() {
            drain_bounded(&mut field, &mut bytes_seen).await?;
            continue;
        }
        // RFC 6266: `filename*=` (decoded) takes precedence over `filename=`.
        // Multer surfaces the plain `filename=` first, so the raw header is
        // read for `filename*=` before falling back (GH #90).
        let filename =
            filename_star_from_headers(&field).or_else(|| field.file_name().map(str::to_string));
        match filename {
            Some(f) if !f.is_empty() => {
                // v1 stores the sanitized basename, not the bytes
                // (FileUpload contract, GH #90): drain to advance the stream.
                drain_bounded(&mut field, &mut bytes_seen).await?;
                out.insert(name, sanitize_filename(&f));
            }
            Some(_) => {
                // Empty filename (no file chosen) → empty value so `required`
                // validation fires instead of treating it as missing.
                drain_bounded(&mut field, &mut bytes_seen).await?;
                out.insert(name, String::new());
            }
            None => {
                // Text fields buffer (extractor-bounded); the read joins the
                // same counter so the backstop sees the per-request total.
                let text = field.text().await?;
                count_form_bytes(&mut bytes_seen, text.len())?;
                out.insert(name, text);
            }
        }
    }
    Ok(out)
}

/// Drain one multipart field chunk-by-chunk, accounting every byte against
/// [`MAX_FORM_BYTES`] (GH #149): enforcement normally happens in the
/// extractor (`BodyLimit` wraps the multipart stream), but the drain owns its
/// own counter so an over-cap upload 413s here too instead of holding the
/// handler.
async fn drain_bounded(
    field: &mut topcoat::router::content::multipart::Field<'_>,
    bytes_seen: &mut usize,
) -> Result<(), topcoat::Error> {
    while let Some(chunk) = field.chunk().await? {
        count_form_bytes(bytes_seen, chunk.len())?;
    }
    Ok(())
}

/// Account one drained chunk against the form-body cap (GH #149). Extracted
/// so the 413 mapping is testable at the boundary without building a
/// multipart body — through the router the extractor's own limit classifies
/// the same body first, so the counter only answers when that limit stops
/// applying (defense-in-depth alongside `BodyLimit`, never a competing cap).
fn count_form_bytes(bytes_seen: &mut usize, chunk_len: usize) -> Result<(), topcoat::Error> {
    // checked_add: the accumulator cannot overflow a usize at real chunk
    // sizes, but wrapping would silently disable the cap in release builds.
    let Some(total) = bytes_seen.checked_add(chunk_len) else {
        return Err(topcoat::router::error::content_too_large().into());
    };
    *bytes_seen = total;
    if *bytes_seen > MAX_FORM_BYTES {
        return Err(topcoat::router::error::content_too_large().into());
    }
    Ok(())
}

/// RFC 5987 `filename*=` from a field's raw `Content-Disposition` header.
fn filename_star_from_headers(
    field: &topcoat::router::content::multipart::Field<'_>,
) -> Option<String> {
    let raw = field
        .headers()
        .get(http::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    raw.split(';').find_map(|seg| {
        let seg = seg.trim();
        seg.get(..10)
            .filter(|h| h.eq_ignore_ascii_case("filename*="))
            .and_then(|_| decode_rfc5987(seg[10..].trim()))
    })
}

/// Pure urlencoded half of [`parse_form_values`] (GH #90) — testable without
/// a request. Rejects bodies over `MAX_FORM_BYTES` with 413. Multipart
/// never reaches here: it streams via [`parse_multipart_values`], where a
/// missing boundary is a 400 and an over-limit body a 413 (both classified
/// by the extractor).
fn form_values_from_request_parts(
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<HashMap<String, String>, topcoat::Error> {
    if bytes.len() > MAX_FORM_BYTES {
        return Err(topcoat::router::error::content_too_large().into());
    }
    debug_assert!(
        content_type.is_none_or(|ct| !is_multipart_content_type(ct)),
        "multipart must stream via parse_multipart_values, not buffer here (GH #90)"
    );
    Ok(form_values_from_bytes(bytes))
}

/// Max form/multipart body accepted (GH #90): 10 MiB. v1 keeps filenames only.
const MAX_FORM_BYTES: usize = 10 * 1024 * 1024;

/// Sanitize a client-supplied filename to a basename (GH #90).
///
/// Strips directory components (`../../etc/passwd` → `passwd`,
/// `/abs/path` → `path`, `C:\fakepath\x` → `x`), trims whitespace, drops
/// control chars, and caps length at 255 bytes. Empty stays empty so
/// `required` validation fires.
///
/// Names that could never be a safely persisted file are rejected to empty
/// (GH #149, before persistence lands): `.` and `..`, and Windows reserved
/// device names (`con`, `nul`, `com1` — also with an extension, and
/// case-insensitive). v1 stores only the basename `String` and never touches
/// the filesystem, so today this is latent; the required validation then
/// surfaces the empty value as an inline form error (on edit, the
/// untouched-file backfill preserves the stored value instead — an
/// explicitly rejected name falls back to "keep", GH #90).
fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "." || trimmed == ".." || is_windows_reserved_name(trimmed) {
        return String::new();
    }
    // Cap at 255 bytes (common filename limit), preserving the tail. The cut
    // point is walked forward to a char boundary: slicing a multibyte char
    // would panic (a >255-byte non-ASCII filename is attacker-controlled).
    if trimmed.len() > 255 {
        let mut start = trimmed.len() - 255;
        while !trimmed.is_char_boundary(start) {
            start += 1;
        }
        trimmed[start..].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Windows reserved device names (GH #149): the stem before the first dot is
/// reserved case-insensitively — `con`, `nul`, `aux`, `prn`, `com1`–`com9`,
/// `lpt1`–`lpt9` — so `con.txt` cannot become a persisted basename either.
fn is_windows_reserved_name(name: &str) -> bool {
    let stem = match name.split_once('.') {
        Some((stem, _)) => stem,
        None => name,
    };
    let stem = stem.to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let Some(n) = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
    else {
        return false;
    };
    n.parse::<u8>().is_ok_and(|n| (1..=9).contains(&n))
}

/// Decode an RFC 5987/6266 `filename*=UTF-8''...` value (GH #90).
///
/// Only UTF-8 is supported; other charsets yield `None` so the caller falls
/// back to `filename=`. Malformed percent sequences fail the whole value
/// rather than lossy-mangling the stored name.
fn decode_rfc5987(value: &str) -> Option<String> {
    let (charset, rest) = value.split_once('\'')?;
    let (_lang, encoded) = rest.split_once('\'')?;
    if !charset.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Pure half of [`parse_form_values`] — testable without a request.
fn form_values_from_bytes(bytes: &[u8]) -> HashMap<String, String> {
    form_urlencoded::parse(bytes).into_owned().collect()
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
fn list_url(cx: &Cx, slug: &str) -> String {
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

/// Shared create/edit page shell (GH #73 multipart enctype, CSRF hidden
/// input, inline error slot). Title and submit label are the only deltas.
async fn render_form_page<'a, R: Resource>(
    cx: &'a Cx,
    title: String,
    submit_label: &'static str,
    values: &HashMap<String, String>,
    errors: &HashMap<String, Vec<String>>,
) -> Result<BoxView<'a>> {
    let schema = R::form(cx);
    let form_html = schema.render_with(cx, values, errors).await?;
    let action = topcoat::router::request::uri(cx).path().to_string();
    // Browsers only send `<input type="file">` content as multipart (GH #73).
    let enctype: Option<String> = schema
        .has_file_upload()
        .then(|| "multipart/form-data".to_string());
    let csrf = crate::csrf::current_token(cx);
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(argentum_ui::page_title((title.clone())))
            argentum_ui::page_content(
                <form
                    method="post"
                    action=(action)
                    enctype=(enctype)
                    class="flex flex-col gap-4"
                >
                    <input type="hidden" name="csrf_token" value=(csrf)>
                    (form_html)
                    <div class="flex gap-2">
                        argentum_ui::button(
                            variant: argentum_ui::ButtonVariant::Primary,
                            attrs: attributes! { type="submit" },
                            (submit_label)
                        )
                        <a
                            href=(list_url(cx, &R::slug()))
                            class="inline-flex items-center justify-center rounded-md border border-border bg-background px-4 py-2 text-sm"
                        >
                            "Cancel"
                        </a>
                    </div>
                </form>
            )
        )
    }
    .boxed())
}

/// Create page GET.
fn resource_create<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_create(cx) {
            return Err(forbidden().into());
        }
        crate::csrf::ensure_token(cx);
        let html = render_form_page::<R>(
            cx,
            format!("Create {}", R::navigation_label()),
            "Create",
            &HashMap::new(),
            &HashMap::new(),
        )
        .await?;
        Ok(html)
    })))
}

/// Reject POST keys no declared Schema input owns (GH #89 mass-assignment
/// allow-list). `csrf_token` is a handler key, not a field, so it is filtered
/// before the check, as are `clear_<field>` flags for declared `FileUpload`
/// fields (GH #90 explicit-clear convention — `truthy`, GH #148); absent keys are fine
/// (present-keys-only updates), unknown keys are a 400 — silently ignoring
/// `role`/`tenant_id` smuggling is what the old code did, and a generic
/// record fn iterating `values` would promote them to client-controlled
/// writes.
fn reject_unknown_form_keys(
    schema: &crate::schema::Schema,
    values: &HashMap<String, String>,
) -> Result<(), topcoat::Error> {
    // One transport-key vocabulary (GH #148): the same `strip_transport_keys`
    // the record fns benefit from defines which keys the framework owns, so
    // the allow-list and the strip cannot drift apart.
    let mut filtered = values.clone();
    strip_transport_keys(schema, &mut filtered);
    let unknown = schema.unknown_keys(&filtered);
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(topcoat::router::error::bad_request(format!(
            "unknown field(s): {}",
            unknown.join(", ")
        ))
        .into())
    }
}

/// The one boolean-vocabulary check for framework form flags (GH #148):
/// `confirm=1|true`, `clear_<field>=1|true`. `yes` used to be a delete-only
/// extra; one vocabulary instead of two per-handler sets.
fn truthy(v: &str) -> bool {
    v == "1" || v == "true"
}

/// Strip framework transport keys from the submitted values before any
/// record fn sees them (GH #148): `csrf_token` and the `clear_<field>`
/// flags are handler keys, not writable fields — a generic `Resource` impl
/// iterating `values` (the exact threat model in the `unknown_keys` docs)
/// must not receive them as writes. The framework strips once here, not
/// per-app convention.
fn strip_transport_keys(schema: &crate::schema::Schema, values: &mut HashMap<String, String>) {
    let declared: std::collections::HashSet<String> = schema.field_names().into_iter().collect();
    // A schema field literally named `csrf_token` (or `clear_<upload>`) is a
    // misconfiguration that would silently swallow its own value here — the
    // declared-name check keeps such a field's value flowing (the collision
    // is a build-time bug, not a transport key).
    values.retain(|k, _| {
        if k == crate::csrf::FIELD_NAME {
            return declared.contains(k.as_str());
        }
        match k.strip_prefix("clear_") {
            Some(field) if schema.file_uploads().contains_key(field) => {
                declared.contains(k.as_str())
            }
            _ => true,
        }
    });
}

/// Create page POST.
/// App-side uniqueness check over the form's `unique()`-marked text inputs.
///
/// Generic over every marked field — the previous version was hard-coded to
/// `email` with a dead full-table query behind it (GH #75 residue). Queries
/// through `Resource::query` (the tenancy seam, ADR-0002) and returns
/// `field_name → ["<Label> has already been taken"]` per duplicated value.
///
/// `current` holds the record's own hydrated values on edit: a field whose
/// submitted value is unchanged belongs to this record and is skipped.
///
/// Known limits (GH #88, upstream gap #117): races with concurrent
/// inserts (only a driver predicate closes it); the check is tenant-scoped via
/// `R::query` while DB `#[unique]` is global, so cross-tenant duplicates 500;
/// empty values are skipped (pair `unique()` with `required()` or normalize
/// `""` vs `NULL` in the record fn); `unique()` exists on `TextInput` only,
/// composite uniques are not covered.
async fn check_unique<R: Resource>(
    cx: &Cx,
    schema: &crate::schema::Schema,
    values: &HashMap<String, String>,
    current: &HashMap<String, String>,
    ex: &mut dyn toasty::Executor,
) -> HashMap<String, Vec<String>> {
    let mut errors: HashMap<String, Vec<String>> = HashMap::new();
    for (name, input) in schema.text_inputs() {
        if !input.is_unique() {
            continue;
        }
        let Some(submitted) = values
            .get(&name)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        // Unchanged on edit → this record's own value, not a duplicate.
        if current.get(&name).map(|s| s.trim().to_string()) == Some(submitted.clone()) {
            continue;
        }
        // Inside the handler's tx (GH #84): the check observes the same
        // snapshot as the write that follows.
        let rows = R::query(cx)
            .filter(input.eq_filter::<R::Model>(submitted))
            .limit(1)
            .exec(&mut *ex)
            .await;
        if matches!(rows, Ok(rows) if !rows.is_empty()) {
            errors.insert(
                name,
                vec![format!("{} has already been taken", input.label_str())],
            );
        }
    }
    errors
}

fn resource_create_post<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_create(cx) {
            return Err(forbidden().into());
        }
        let mut values = parse_form_values(cx, body).await?;
        crate::csrf::verify(cx, &values)?;
        let schema = R::form(cx);
        reject_unknown_form_keys(&schema, &values)?;
        // Transport keys never reach the record fn (GH #148): a generic impl
        // iterating `values` must not see `csrf_token`/`clear_*` as writable
        // fields — the framework strips them once, not per-app convention.
        strip_transport_keys(&schema, &mut values);
        let mut errors = schema.validate_async(cx, &values).await;
        // Framework-owned transaction (GH #84): opened only after
        // validation — `validate_async` relationship loaders run on their
        // own handle, which would block on the pool while the tx holds it
        // (see `db` pool discipline). The unique check and the write then
        // observe one snapshot and commit atomically. Dropping `tx`
        // without commit (validation errors, policy denials) rolls back.
        let mut db = db(cx);
        let mut tx = db.transaction().await.map_err(topcoat::Error::from)?;
        // App-side unique check over every `unique()`-marked input — the only
        // error layer until toasty exposes a unique-violation predicate
        // (upstream gap #117; never string-match driver error messages).
        for (name, errs) in check_unique::<R>(cx, &schema, &values, &HashMap::new(), &mut tx).await
        {
            errors.entry(name).or_default().extend(errs);
        }
        if !errors.is_empty() {
            // Drop the tx before rendering (GH #84): the re-rendered form
            // reloads relationship options on its own handle, which would
            // block on the pool while the tx holds it.
            drop(tx);
            let html = render_form_page::<R>(
                cx,
                format!("Create {}", R::navigation_label()),
                "Create",
                &values,
                &errors,
            )
            .await?;
            return Ok(html);
        }
        // Attempt creation via Resource hook, inside the tx.
        match R::create_record(cx, values.clone(), &mut tx).await {
            Ok(()) => {
                tx.commit().await.map_err(topcoat::Error::from)?;
                // Post/Redirect/Get: the browser follows with a GET, and the
                // flash cookie rides the error response (Topcoat flushes
                // `Set-Cookie` on `Err` too, topcoat#408).
                set_notification(cx, Notification::success("Created"));
                Err(see_other(list_url(cx, &R::slug())).into())
            }
            // A unique violation that slipped past the app-side check (a
            // concurrent insert) surfaces as an error, not a string-matched
            // inline message (upstream gap #117).
            Err(e) => Err(e),
        }
    })))
}

/// Fetch one record by its URL `id` through the tenancy-scoped query seam.
///
/// The string id is parsed against the model's primary-key type and the PK
/// filter is ANDed onto [`Resource::query`](crate::resource::Resource::query)
/// (ADR-0002), so tenancy/soft-delete scoping holds. Replaces the #75 item-1
/// pattern of fetching every row and matching `Table::key_for` in memory —
/// O(N) rows per edit/delete, leaking the whole table before the policy
/// check.
///
/// A malformed or unknown id maps to 404, not a query error.
///
/// Runs on the caller's executor: mutation handlers pass the open framework
/// transaction (GH #84) so the fetched snapshot is the checked snapshot.
async fn find_by_key<R: Resource>(
    cx: &Cx,
    id: &str,
    ex: &mut dyn toasty::Executor,
) -> Result<R::Model> {
    let Some(expr) = crate::schema::pk_eq_expr::<R::Model>(id) else {
        // Composite PKs have no URL representation (GH #95): fail loudly so
        // the misconfiguration surfaces instead of 404ing every id.
        if crate::schema::pk_is_composite::<R::Model>() {
            tracing::error!(
                resource = R::slug(),
                "composite primary key has no URL representation"
            );
            return Err(topcoat::Error::from(std::io::Error::other(format!(
                "resource '{}' has a composite primary key, which has no URL representation (GH #95)",
                R::slug()
            ))));
        }
        return Err(topcoat::router::error::not_found().into());
    };
    R::query(cx)
        .filter(expr)
        .first()
        .exec(&mut *ex)
        .await
        .map_err(topcoat::Error::from)?
        .ok_or_else(topcoat::router::error::not_found)
        .map_err(Into::into)
}

/// Edit page GET — hydrates form from model via Resource::query seam.
fn resource_edit<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        let mut db = db(cx);
        let record = find_by_key::<R>(cx, &id, &mut db).await?;
        if !R::can_view(cx, &record) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &record) {
            return Err(forbidden().into());
        }
        crate::csrf::ensure_token(cx);
        let values = R::hydrate_form_values(&record);
        let html = render_form_page::<R>(
            cx,
            format!("Edit {}", R::navigation_label()),
            "Save",
            &values,
            &HashMap::new(),
        )
        .await?;
        Ok(html)
    })))
}

/// Edit page POST — validates, checks `can_view` + `can_update`, mutates via Update projection.
///
/// Requires both `can_view` and `can_update` (matching GET, GH #86 deny-by-default):
/// a view-denied but writable record must not be mutable by direct POST.
fn resource_edit_post<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        let mut values = parse_form_values(cx, body).await?;
        crate::csrf::verify(cx, &values)?;
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        // Advisory load on a pooled handle (GH #86): feeds hydration and the
        // pre-validation file backfill below. The body is already parsed and
        // CSRF-verified (GH #144), so the load never runs for a forged POST.
        // The authoritative load + policy check happens inside the framework
        // transaction — validation (`validate_async` relationship loaders)
        // runs on its own handle and must never execute while the tx holds
        // the pool (see `db` pool discipline).
        let mut db0 = db(cx);
        let advisory = find_by_key::<R>(cx, &id, &mut db0).await?;
        if !R::can_view(cx, &advisory) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &advisory) {
            return Err(forbidden().into());
        }
        let schema = R::form(cx);
        reject_unknown_form_keys(&schema, &values)?;
        // Unique check excludes this record's own unchanged values.
        let current = R::hydrate_form_values(&advisory);
        // Untouched file inputs preserve the stored path (GH #90): the edit
        // form renders an empty file input (browsers never pre-fill it), so
        // an empty submit means "keep", not "clear" — without this the
        // required check rejects untouched edits and optional uploads get
        // blanked. An explicit `clear_<field>=1` opts back into clearing
        // (apps render their own checkbox; a first-class control is future
        // work).
        for name in schema.file_uploads().keys() {
            let cleared = values
                .get(&format!("clear_{name}"))
                .is_some_and(|v| truthy(v));
            let empty = values
                .get(name)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true);
            if !cleared && empty && current.get(name).is_some_and(|v| !v.trim().is_empty()) {
                values.insert(name.clone(), current[name].clone());
            }
        }
        // Transport keys never reach the record fn (GH #148) — see create.
        strip_transport_keys(&schema, &mut values);
        let mut errors = schema.validate_async(cx, &values).await;
        // Authoritative load inside the framework transaction (GH #84, #86):
        // policy is checked on this snapshot and the same record flows into
        // the write — never a silent re-load outside the checked snapshot.
        let mut db = db(cx);
        let mut tx = db.transaction().await.map_err(topcoat::Error::from)?;
        let record = find_by_key::<R>(cx, &id, &mut tx).await?;
        if !R::can_view(cx, &record) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &record) {
            return Err(forbidden().into());
        }
        for (name, errs) in check_unique::<R>(cx, &schema, &values, &current, &mut tx).await {
            errors.entry(name).or_default().extend(errs);
        }
        if !errors.is_empty() {
            // Drop the tx before rendering (GH #84): see create POST.
            drop(tx);
            let html = render_form_page::<R>(
                cx,
                format!("Edit {}", R::navigation_label()),
                "Save",
                &values,
                &errors,
            )
            .await?;
            return Ok(html);
        }
        match R::update_record(cx, record, values.clone(), &mut tx).await {
            Ok(()) => {
                tx.commit().await.map_err(topcoat::Error::from)?;
                set_notification(cx, Notification::success("Updated"));
                Err(see_other(list_url(cx, &R::slug())).into())
            }
            // A unique violation that slipped past the app-side check (a
            // concurrent update) surfaces as an error, not a string-matched
            // inline message (upstream gap #117).
            Err(e) => Err(e),
        }
    })))
}

/// Delete action POST — confirmation-marked, policy-checked, and run in the
/// framework transaction (GH #84): the checked record flows into the write.
///
/// The confirmation is the row's alert dialog on the list page (GH #151): the
/// Delete link opens `?delete=<key>` and the dialog's form POSTs here with
/// `confirm=1`. Authentication comes before any DB work (GH #144): the CSRF
/// check and the confirmation marker run first, so a forged POST answers 403
/// without opening a transaction, holding a pooled connection across the body
/// read, or probing record existence (create/bulk-delete ordering, GH #84).
/// The dialog itself is deliberately fetch-free and policy-blind: it carries
/// no record data and embeds only the caller's own CSRF token, and the
/// policy/tenancy checks run against the loaded record here.
fn resource_delete<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::<_, BoxView<'_>>::new(
        async move {
            enforce_auth(cx)?;
            enforce_tenant::<R>(cx)?;
            let values = parse_form_values(cx, body).await?;
            crate::csrf::verify(cx, &values)?;
            let confirmed = values.get("confirm").is_some_and(|v| truthy(v));
            if !confirmed {
                // The confirmation UI is the list-page alert dialog (GH #151):
                // the row link opens `?delete=<key>` and the dialog's form carries
                // `confirm=1`. This route only accepts that confirmed POST, so a
                // missing marker is a malformed client, not a user path.
                return Err(
                    topcoat::router::error::bad_request("delete requires confirmation").into(),
                );
            }
            // Confirmed and authenticated: open the transaction only now (GH
            // #144), fetch through the tenancy seam, check Policy against the
            // loaded record, and delete inside the tx — commit makes the checked
            // delete durable, any error rolls it back (GH #84).
            let mut db = db(cx);
            let mut tx = db.transaction().await.map_err(topcoat::Error::from)?;
            let id = topcoat::router::path_param_segment(cx, "id").to_string();
            let record = find_by_key::<R>(cx, &id, &mut tx).await?;
            if !R::can_delete(cx, &record) {
                return Err(forbidden().into());
            }
            R::delete_record(cx, record, &mut tx).await?;
            tx.commit().await.map_err(topcoat::Error::from)?;
            set_notification(cx, Notification::success("Deleted"));
            Err(see_other(list_url(cx, &R::slug())).into())
        },
    )))
}

/// Bulk delete POST — ids via `ids` form field (comma-separated).
///
/// Identity is the typed PK fetch alone (GH #85): the display closure
/// `Table::id` is never re-matched, so non-canonical keys (uppercase UUID,
/// email key) cannot 404 a batch whose rows exist. Bounded by
/// `MAX_BULK_IDS` so the `IN` list cannot be amplified into a DoS.
/// Fetch, policy checks, and deletes share one framework transaction
/// (GH #84): a mid-loop failure deletes zero rows.
fn resource_bulk_delete<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::<_, BoxView<'_>>::new(
        async move {
            enforce_auth(cx)?;
            enforce_tenant::<R>(cx)?;
            let values = parse_form_values(cx, body).await?;
            crate::csrf::verify(cx, &values)?;
            let ids_raw = values.get("ids").cloned().unwrap_or_default();
            let ids = parse_bulk_ids(&ids_raw, MAX_BULK_IDS);
            if ids.is_empty() {
                // No ids is a validation miss, not a raw 400 page (GH #151):
                // the bulk bar disables its submit until a row is checked, so
                // only a crafted (or stale) POST gets here — answer like any
                // other mutation, with the list and the reason.
                set_notification(cx, Notification::error("Select at least one row to delete"));
                return Err(see_other(list_url(cx, &R::slug())).into());
            }
            if ids.len() > MAX_BULK_IDS {
                return Err(topcoat::router::error::bad_request(format!(
                    "too many ids (max {MAX_BULK_IDS})"
                ))
                .into());
            }
            // Fetch only the requested rows through the tenancy-scoped seam:
            // one `pk IN (…)` query replaces the #75 item-1
            // fetch-everything-then-match loop. A malformed id cannot exist and
            // maps to 404; a missing/wrong-tenant id makes the fetch come back
            // short and 404s as well.
            let keys: Vec<&str> = ids.iter().map(String::as_str).collect();
            let Some(pk_filter) = crate::schema::pk_in_expr::<R::Model>(&keys) else {
                if crate::schema::pk_is_composite::<R::Model>() {
                    tracing::error!(
                        resource = R::slug(),
                        "composite primary key has no URL representation"
                    );
                    return Err(topcoat::Error::from(std::io::Error::other(format!(
                        "resource '{}' has a composite primary key, which has no URL representation (GH #95)",
                        R::slug()
                    ))));
                }
                return Err(topcoat::router::error::not_found().into());
            };
            let mut db = db(cx);
            let mut tx = db.transaction().await.map_err(topcoat::Error::from)?;
            let rows = R::query(cx)
                .filter(pk_filter)
                .exec(&mut tx)
                .await
                .map_err(topcoat::Error::from)?;
            if rows.len() != ids.len() {
                return Err(topcoat::router::error::not_found().into());
            }
            for rec in &rows {
                if !R::can_delete(cx, rec) {
                    return Err(forbidden().into());
                }
            }
            // All checks passed — perform bulk delete inside the tx, then
            // commit once. Any error drops `tx` uncommitted: zero rows
            // deleted, never half-applied.
            R::bulk_delete_records(cx, rows, &mut tx).await?;
            tx.commit().await.map_err(topcoat::Error::from)?;
            set_notification(cx, Notification::success("Bulk deleted"));
            Err(see_other(list_url(cx, &R::slug())).into())
        },
    )))
}

/// Max ids accepted by bulk delete (GH #85): bounds the `IN` list.
const MAX_BULK_IDS: usize = 400;

/// Max rows an export will materialize (GH #94): the filtered query carries
/// `limit(MAX_EXPORT_ROWS + 1)` and anything past the cap is a 413, so a
/// 100k-row table stays bounded in memory instead of buffering `Vec<Model>` +
/// `String` without end.
const MAX_EXPORT_ROWS: usize = 10_000;

/// Reject an export whose filtered query returned one row past the cap
/// (GH #94). Extracted from the handler so the 413 mapping is testable at the
/// boundary without materializing 10k rows in a test database.
fn enforce_export_cap<T>(rows: Vec<T>) -> Result<Vec<T>, topcoat::Error> {
    if rows.len() > MAX_EXPORT_ROWS {
        Err(topcoat::router::error::content_too_large().into())
    } else {
        Ok(rows)
    }
}

/// Filter an export's rows to those the caller may view, then apply the cap
/// (GH #86, GH #145): the 413 reflects what the caller may actually receive —
/// never the pre-visibility count, which would both 413 tables whose visible
/// rows fit and leak the existence/count of denied rows.
fn filter_then_cap<T>(
    mut rows: Vec<T>,
    can_view: impl Fn(&T) -> bool,
) -> Result<Vec<T>, topcoat::Error> {
    rows.retain(|r| can_view(r));
    enforce_export_cap(rows)
}

/// The longest slug a `Content-Disposition` filename keeps (GH #145): the
/// header value stays bounded even for an oversized override.
const MAX_EXPORT_FILENAME_LEN: usize = 100;

/// Sanitize the export's `Content-Disposition` filename (GH #145): `slug()`
/// is an overridable free-form `String`, and Topcoat route validation accepts
/// quote and CR/LF segments, so a hostile override would otherwise split the
/// response header. Quote, backslash, and control characters are dropped and
/// the length is capped before the `.csv` suffix. Non-ASCII overrides pass
/// through as obs-text (browsers render them; an RFC 6266 `filename*` is
/// future work).
fn export_filename(slug: &str) -> String {
    let safe: String = slug
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .take(MAX_EXPORT_FILENAME_LEN)
        .collect();
    if safe.is_empty() {
        "export.csv".to_string()
    } else {
        format!("{safe}.csv")
    }
}

/// `?bom=1` opts into a UTF-8 BOM prefix on the CSV body for Excel (GH #94).
fn export_wants_bom(cx: &Cx) -> bool {
    let Some(parts) = topcoat::context::try_request_context::<http::request::Parts>(cx) else {
        return false;
    };
    let Some(query) = parts.uri.query() else {
        return false;
    };
    form_urlencoded::parse(query.as_bytes()).any(|(k, v)| k == "bom" && v == "1")
}

/// Parse + dedupe bulk `ids` while preserving order, so a repeated id can't
/// make the fetched-rows count check misfire.
///
/// `max` bounds the parse itself, not just the final list (GH #85): a 10 MiB
/// body of distinct ids stops at `max + 1` entries (which the handler then
/// rejects with 400) instead of allocating millions of strings while the
/// `MAX_BULK_IDS` check waits for the parse to finish. Deduping uses a set —
/// the previous `Vec::contains` scan was quadratic.
///
/// Known limit (GH #85): the split happens after url-decoding, so a
/// `String`-PK id containing a literal comma (`%2C`) splits into phantom
/// ids and the batch 404s. Comma-bearing string PKs need a different
/// transport (future work); all other PK types are comma-free.
fn parse_bulk_ids(raw: &str, max: usize) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if seen.insert(s) {
            ids.push(s.to_string());
            if ids.len() > max {
                break;
            }
        }
    }
    ids
}

/// CSV export — reuses `Resource::query` + `Table` filters/sort, downloads `text/csv`.
///
/// The filtered query is capped at [`MAX_EXPORT_ROWS`] + 1 rows at the query
/// layer so a 100k-row table cannot OOM the handler; visibility is applied
/// before the cap (`filter_then_cap`, GH #145) so the 413 reflects what the
/// caller may receive, formula cells are defused per OWASP in
/// [`Table::to_csv`], and `?bom=1` prepends a UTF-8 BOM for Excel interop
/// (GH #94).
fn resource_export<R: Resource>(cx: &Cx, _body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_view_any(cx) {
            return Err(forbidden().into());
        }
        let state = TableState::from_cx(cx);
        let table = R::table(cx);
        // Fail closed on unapplied filters (GH #93): a typo'd `?filters=`
        // must not silently export the unfiltered table.
        if !table.unapplied_filters(&state).is_empty() {
            return Err(topcoat::router::error::bad_request(format!(
                "invalid filters: {}",
                table
                    .unapplied_filters(&state)
                    .iter()
                    .map(|(pair, reason)| format!("{pair} ({reason})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .into());
        }
        let mut query = R::query(cx);
        if let Some(term) = &state.search
            && let Some(expr) = table.search_expr(term)
        {
            query = query.filter(expr);
        }
        if let Some(expr) = table.filter_expr(&state) {
            query = query.filter(expr);
        }
        for ord in table.order_bys_for_state(&state) {
            query = query.order_by(ord);
        }
        // Bound the export at the query layer (GH #94): the DB returns at
        // most one row past the cap, so memory stays bounded.
        query = query.limit(MAX_EXPORT_ROWS + 1);
        let mut db = db(cx);
        let rows: Vec<R::Model> = query.exec(&mut db).await.map_err(topcoat::Error::from)?;
        // Visibility first, cap second (GH #86, GH #145) — see
        // `filter_then_cap` for why the cap counts only receivable rows.
        // Bounded over-fetch (the issue's accepted alternative): a 200 holds
        // the visible rows of the first MAX+1 fetched rows, so when denied
        // rows interleave in query order, visible rows past the window are
        // not exported. No truncation signal is emitted for that case: any
        // window-full marker would leak the pre-visibility row count, which
        // the same acceptance criterion forbids ("no count leak").
        let rows = filter_then_cap(rows, |r| R::can_view(cx, r))?;
        // Build TablePage without pagination for CSV (all rows)
        let page: TablePage<R::Model> = rows.into();
        let mut csv = table.to_csv(&page);
        // Opt-in BOM for Excel (GH #94): `?bom=1` prepends U+FEFF so
        // non-ASCII cells open correctly; default stays BOM-free so existing
        // clients/tests see a plain UTF-8 body.
        if export_wants_bom(cx) {
            csv.insert(0, '\u{FEFF}');
        }
        let filename = export_filename(&R::slug());
        let res = http::Response::builder()
            .status(200)
            .header(http::header::CONTENT_TYPE, "text/csv; charset=utf-8")
            .header(
                http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
            .body(Body::from(csv))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(res)
    })
}

/// The panel root: a temporary redirect to the first declared resource's
/// list, so the mount point is never a dead URL (until Dashboards exist,
/// GH #38). Filament registers a Dashboard page here.
fn panel_root_redirect(cx: &Cx, _body: Body) -> RouteFuture<'_> {
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
    fn brand_trims_name_and_ignores_blank_logo() {
        assert_eq!(Brand::new("  Acme  ").name, "Acme");
        assert_eq!(Brand::new("Acme").logo, None);
        assert_eq!(
            Brand::new("Acme").logo("  /logo.svg  ").logo.as_deref(),
            Some("/logo.svg")
        );
        // Blank logos fall back to the name-only render (GH #102).
        assert_eq!(Brand::new("Acme").logo("   ").logo, None);
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

    /// The live-search shard answers the gate before the registry lookup
    /// (GH #146): an unauthenticated probe cannot distinguish a registered
    /// slug from an unregistered one.
    #[tokio::test]
    async fn search_shard_answers_auth_before_the_registry_lookup() {
        use topcoat::context::CxTestBuilder;
        use topcoat::router::response::IntoResponse;

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

        // A registry that really knows the `users` slug, so the known-path
        // probe is a resolution the gate must preempt.
        let (parts, ()) = http::Request::builder()
            .method(http::Method::POST)
            .uri(crate::auth::RUNTIME_PREFIX)
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(crate::Auth::password())
            .app_context(SearchRegistry(HashMap::from([(
                "users".to_string(),
                search_handler_for::<DummyResource>(),
            )])))
            .build();

        // The gate's answer comes before the lookup: both a registered and an
        // unregistered slug answer 401 identically (no 404 oracle).
        let unknown = match search_entry(&cx, "not-a-slug") {
            Ok(_) => panic!("unauthenticated probe must not resolve an entry"),
            Err(err) => err,
        };
        let registered = match search_entry(&cx, "users") {
            Ok(_) => panic!("an unauthenticated probe must never reach the registry"),
            Err(err) => err,
        };
        let unknown_status = unknown
            .into_response(&cx)
            .expect("gate answer renders")
            .status();
        let registered_status = registered
            .into_response(&cx)
            .expect("gate answer renders")
            .status();
        assert_eq!(
            unknown_status, registered_status,
            "unauthenticated probes must not distinguish registered slugs"
        );
        assert_eq!(
            registered_status,
            http::StatusCode::UNAUTHORIZED,
            "runtime probes answer 401 (ADR-0013), got {registered_status}"
        );

        // Auth disabled (the shard's own lookup is what remains): a
        // registered path resolves and an unknown path is a plain 404 again.
        let (parts, ()) = http::Request::builder()
            .method(http::Method::POST)
            .uri(crate::auth::RUNTIME_PREFIX)
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(crate::Auth::disabled())
            .app_context(SearchRegistry(HashMap::from([(
                "users".to_string(),
                search_handler_for::<DummyResource>(),
            )])))
            .build();
        assert!(
            search_entry(&cx, "users").is_ok(),
            "with auth disabled a registered path resolves through the lookup"
        );
        let err = match search_entry(&cx, "not-a-slug") {
            Ok(_) => panic!("an unregistered path must not resolve"),
            Err(err) => err,
        };
        assert!(
            err.downcast_ref::<topcoat::router::error::NotFoundError>()
                .is_some(),
            "with auth disabled the unknown path is a plain 404, got {err}"
        );
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

    #[tokio::test]
    async fn edit_post_requires_can_view_as_well_as_can_update() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct ViewDeniedResource;
        impl Resource for ViewDeniedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                false
            }
            fn can_update(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
            async fn update_record(
                _cx: &Cx,
                _record: Dummy,
                _values: HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<ViewDeniedResource>()
            .auth(crate::Auth::disabled())
            .build();
        let url = format!("/admin/dummies/{}/edit", row.id);
        // GET already required both; POST must match (GH #86).
        let get = router
            .handle(
                http::Request::builder()
                    .uri(&url)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(get.status(), http::StatusCode::FORBIDDEN);
        // Valid CSRF token still 403 on policy (not on CSRF).
        let token = uuid::Uuid::new_v4().to_string();
        let post = router
            .handle(
                http::Request::builder()
                    .uri(&url)
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("name=Ada&csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            post.status(),
            http::StatusCode::FORBIDDEN,
            "view-denied edit POST must not mutate"
        );
        // Missing token is 403 even before policy (GH #99).
        let no_token = router
            .handle(
                http::Request::builder()
                    .uri(&url)
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(Body::from("name=Ada"))
                    .unwrap(),
            )
            .await;
        assert_eq!(no_token.status(), http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn parse_bulk_ids_dedupes_and_trims() {
        assert!(parse_bulk_ids("", MAX_BULK_IDS).is_empty());
        assert_eq!(
            parse_bulk_ids("a, b ,a,, c", MAX_BULK_IDS),
            vec!["a", "b", "c"]
        );
        // The cap bounds the parse too: stop at max + 1 for the handler's 400.
        assert_eq!(parse_bulk_ids("a,b,c,d,e", 3).len(), 4);
    }

    #[tokio::test]
    async fn bulk_delete_caps_ids_and_ignores_display_key() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct UpperKeyResource;
        impl Resource for UpperKeyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                // Non-canonical display key (GH #85): bulk must still resolve
                // via the typed PK fetch alone.
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string().to_uppercase())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            async fn bulk_delete_records(
                _cx: &Cx,
                _records: Vec<Dummy>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<UpperKeyResource>()
            .auth(crate::Auth::disabled())
            .build();
        // Canonical lowercase id succeeds despite uppercase Table::id.
        let token = uuid::Uuid::new_v4().to_string();
        let ok = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("ids={}&csrf_token={token}", row.id)))
                    .unwrap(),
            )
            .await;
        assert!(
            ok.status().is_redirection(),
            "PK-authenticated bulk must not 404 on display-key mismatch, got {}",
            ok.status()
        );
        // Over-cap batch is a clear 400 before any DB work.
        let big = (0..(MAX_BULK_IDS + 1))
            .map(|i| format!("00000000-0000-0000-0000-{:012}", i))
            .collect::<Vec<_>>()
            .join(",");
        let capped = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("ids={big}&csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(capped.status(), http::StatusCode::BAD_REQUEST);
        // Missing token is 403 (GH #99).
        let no_token = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(Body::from(format!("ids={}", row.id)))
                    .unwrap(),
            )
            .await;
        assert_eq!(no_token.status(), http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn bulk_delete_mid_loop_failure_deletes_zero_rows() {
        // GH #84 acceptance: fetch, policy checks, and deletes share one
        // framework transaction — an impl that fails halfway rolls everything
        // back instead of half-applying.
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct FlakyBulkResource;
        impl Resource for FlakyBulkResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            async fn bulk_delete_records(
                _cx: &Cx,
                records: Vec<Dummy>,
                ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                // Delete the first row, then blow up: without the
                // framework tx the first delete would stick.
                let first = records.into_iter().next().unwrap();
                Dummy::filter(Dummy::fields().id().eq(first.id))
                    .delete()
                    .exec(&mut *ex)
                    .await
                    .map_err(topcoat::Error::from)?;
                Err(std::io::Error::other("boom").into())
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["one", "two"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let mut db_ids = db.clone();
        let rows = Dummy::all().exec(&mut db_ids).await.unwrap();
        assert_eq!(rows.len(), 2);
        let ids = rows
            .iter()
            .map(|r| r.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let router = Panel::new("admin")
            .app_context(db.clone())
            .resource::<FlakyBulkResource>()
            .auth(crate::Auth::disabled())
            .build();
        let token = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("ids={ids}&csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert!(
            resp.status().is_server_error(),
            "mid-loop failure must error, got {}",
            resp.status()
        );
        let rows = Dummy::all().exec(&mut db_ids).await.unwrap();
        assert_eq!(
            rows.len(),
            2,
            "rollback must leave zero rows deleted, got {}",
            2 - rows.len()
        );
    }

    #[tokio::test]
    async fn live_search_host_and_shard_dispatch() {
        // GH #104: opt-in tables render the signal host (page bodies are
        // hoisted, so signals work there); the slug-dispatched shard serves
        // the grid and 404s unknown paths.
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct LiveResource;
        impl Resource for LiveResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                            d.name.clone()
                        })
                        .searchable()
                        .sortable(),
                    )
                    .paginate(1)
                    .live_search(true)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db.clone())
            .resource::<LiveResource>()
            .auth(crate::Auth::disabled())
            .build();

        // List page carries the live host + GET fallback.
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("data-live-search"),
            "opt-in table must render the shard host, got {html}"
        );
        assert!(
            html.contains("<noscript>"),
            "live table must keep the GET fallback, got {html}"
        );

        // Shard dispatch through the real runtime endpoint (JSON args + the
        // identity header the browser sends): unknown path fails, registered
        // path renders rows and the live controls bound to the caller's
        // signals.
        let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
        let shard_args = |path: &str,
                          q: &str,
                          filters: &str,
                          sort: &str,
                          dir: &str,
                          after: &str,
                          before: &str| {
            format!(
                r#"["{path}",{}, {}, {}, {}, {}, {}, ""]"#,
                sig(1, q),
                sig(2, filters),
                sig(3, sort),
                sig(4, dir),
                sig(5, after),
                sig(6, before)
            )
        };
        async fn call_shard(
            router: &topcoat::router::Router,
            args: String,
        ) -> http::Response<Body> {
            let shard = topcoat::runtime::Shard::id(&table_search);
            router
                .handle(
                    http::Request::builder()
                        .method(http::Method::POST)
                        .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
                        .header(http::header::CONTENT_TYPE, "application/json")
                        .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                        .body(Body::from(format!(r#"{{"args":{args},"signals":{{}}}}"#)))
                        .unwrap(),
                )
                .await
        }
        let nope = call_shard(
            &router,
            shard_args("/admin/nope", "Ada", "", "", "", "", ""),
        )
        .await;
        assert!(
            nope.status().is_client_error(),
            "unknown shard path must fail, got {}",
            nope.status()
        );
        let grid = call_shard(
            &router,
            shard_args("/admin/dummies", "Ada", "", "", "", "", ""),
        )
        .await;
        let status = grid.status();
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes).to_string();
        assert_eq!(status, http::StatusCode::OK, "shard body: {grid_html}");
        assert!(
            grid_html.contains("Ada"),
            "live shard must render matching rows, got {grid_html}"
        );
        // GH #151: the grid's chrome is bound to the signals, so sort/pager
        // interactions re-render in place. `href` stays the no-JS fallback.
        assert!(
            grid_html.contains("data-topcoat-on:click") && grid_html.contains("sort=name"),
            "live grid must bind the sort link and keep its href, got {grid_html}"
        );

        // A direct context for the loader/cursor assertions below.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/dummies")
            .body(())
            .unwrap()
            .into_parts();
        let cx = topcoat::context::CxTestBuilder::new()
            .request_context(parts)
            .app_context(db)
            .build();

        // Cursors are honored as sent (GH #151): the sort/filter/search
        // handlers clear them in the browser, so a live cursor always belongs
        // to the current query; crafting one past a new query is the client's
        // own read-only inconsistency.
        let paged = crate::resource::Table::<Dummy>::r#for(&cx)
            .id(|d: &Dummy| d.id.to_string())
            .columns(crate::resource::TextColumn::r#for(
                Dummy::fields().name(),
                |d: &Dummy| d.name.clone(),
            ))
            .paginate(1);
        for name in ["Bob", "Cara"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut crate::db::db(&cx))
            .await
            .unwrap();
        }
        let page1 =
            load_table_page::<LiveResource>(&cx, &paged, &crate::resource::TableState::default())
                .await
                .unwrap();
        assert_eq!(page1.rows.len(), 1);
        let first_name = page1.rows[0].name.clone();
        let cursor = page1
            .next_cursor
            .clone()
            .expect("page 1 must have a cursor");
        let grid = call_shard(
            &router,
            shard_args("/admin/dummies", "", "", "", "", &cursor, ""),
        )
        .await;
        assert_eq!(grid.status(), http::StatusCode::OK);
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes);
        assert!(
            !grid_html.contains(&first_name),
            "a cursor must continue past page-1 rows, got {grid_html}"
        );
        // The pager renders its next/prev links with handlers too.
        assert!(
            grid_html.contains("data-topcoat-on:click"),
            "live pager must bind its cursor handlers, got {grid_html}"
        );
        // A fresh search with no cursor starts a new result set.
        let grid = call_shard(
            &router,
            shard_args("/admin/dummies", "Bob", "", "", "", "", ""),
        )
        .await;
        assert_eq!(grid.status(), http::StatusCode::OK);
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes);
        assert!(
            grid_html.contains("Bob"),
            "fresh search must match new query, got {grid_html}"
        );
    }

    #[tokio::test]
    async fn read_only_resource_hides_delete_chrome() {
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct ReadOnlyResource;
        impl Resource for ReadOnlyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn deletable() -> bool {
                false
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<ReadOnlyResource>()
            .auth(crate::Auth::disabled())
            .build();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            !html.contains("data-bulk-form") && !html.contains("Bulk Delete"),
            "read-only list must not render bulk chrome, got {html}"
        );
        assert!(
            !html.contains("/delete"),
            "read-only list must not render delete actions, got {html}"
        );
    }

    #[tokio::test]
    async fn export_drops_rows_failing_can_view() {
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct RowPolicyResource;
        impl Resource for RowPolicyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &Dummy) -> bool {
                record.name != "denied"
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["allowed", "denied"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<RowPolicyResource>()
            .auth(crate::Auth::disabled())
            .build();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let csv = String::from_utf8_lossy(&body);
        assert!(
            csv.contains("allowed"),
            "export must keep viewable rows, got {csv}"
        );
        assert!(
            !csv.contains("denied"),
            "export must not exceed row visibility (GH #86), got {csv}"
        );
    }

    /// The GET `?q=` term is clamped like the shard's (GH #148): bounded
    /// echoed state.
    #[test]
    fn from_cx_clamps_the_search_term() {
        use topcoat::context::CxTestBuilder;

        fn state_for(uri: &str) -> TableState {
            let (parts, ()) = http::Request::builder()
                .uri(uri)
                .body(())
                .unwrap()
                .into_parts();
            let cx = CxTestBuilder::new().request_context(parts).build();
            TableState::from_cx(&cx)
        }

        let long = "x".repeat(500);
        let state = state_for(&format!("/admin/users?q={long}"));
        assert_eq!(
            state.search.as_deref().map(str::len),
            Some(crate::resource::MAX_QUERY_TERM),
            "the GET term is clamped to the same bound as the shard"
        );
        // Blank and absent stay None.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users?q=")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert!(TableState::from_cx(&cx).search.is_none());
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert!(TableState::from_cx(&cx).search.is_none());
    }

    /// One boolean vocabulary for framework form flags (GH #148): `1` and
    /// `true` are truthy everywhere (`confirm`, `clear_<field>`); `yes` was a
    /// delete-only extra and is gone.
    #[test]
    fn truthy_accepts_one_vocabulary() {
        assert!(truthy("1") && truthy("true"));
        assert!(!truthy("yes") && !truthy("") && !truthy("on") && !truthy("TRUE"));
    }

    /// Record fns never see framework transport keys (GH #148): the create
    /// POST carries `csrf_token` (and, for file schemas, `clear_<field>`) —
    /// the framework strips them before `create_record`, so a generic impl
    /// iterating `values` cannot treat them as writable fields.
    #[tokio::test]
    async fn create_record_receives_no_transport_keys() {
        use crate::schema::{FileUpload, Schema, TextInput};
        use std::sync::Mutex;

        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
            title: String,
        }

        static RECEIVED: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
        struct CapturingResource;
        impl crate::resource::Resource for CapturingResource {
            type Model = Doc;
            fn slug() -> String {
                "docs".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Doc> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Doc| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Doc::fields().title(),
                        |d: &Doc| d.title.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> Schema {
                Schema::new((
                    TextInput::r#for(Doc::fields().title()),
                    FileUpload::r#for(Doc::fields().path()),
                ))
            }
            async fn create_record(
                _cx: &Cx,
                values: HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> topcoat::Result<()> {
                let mut keys = values.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                RECEIVED.lock().unwrap().push(keys);
                Ok(())
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Doc))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<CapturingResource>()
            .auth(crate::Auth::disabled())
            .build();
        let csrf = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri("/admin/docs/create")
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={csrf}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!(
                        "title=x&path=/tmp/a.bin&csrf_token={csrf}&clear_path=1"
                    )))
                    .unwrap(),
            )
            .await;
        assert!(
            resp.status().is_redirection(),
            "create succeeds, got {} {}",
            resp.status(),
            String::from_utf8_lossy(
                &http_body_util::BodyExt::collect(resp.into_body())
                    .await
                    .unwrap()
                    .to_bytes()
            )
        );
        let received = RECEIVED.lock().unwrap();
        let keys = received.last().expect("create_record ran");
        assert!(
            !keys.contains(&"csrf_token".to_string()) && !keys.contains(&"clear_path".to_string()),
            "transport keys must be stripped before the record fn, got {keys:?}"
        );
        assert_eq!(keys.len(), 2, "declared fields only, got {keys:?}");
    }

    #[test]
    fn export_bom_flag_reads_bom_query_param() {
        use topcoat::context::CxTestBuilder;

        fn cx_for(uri: &str) -> Cx {
            let (parts, ()) = http::Request::builder()
                .uri(uri)
                .body(())
                .unwrap()
                .into_parts();
            CxTestBuilder::new().request_context(parts).build()
        }

        assert!(export_wants_bom(&cx_for("/admin/users/export?bom=1")));
        assert!(!export_wants_bom(&cx_for("/admin/users/export")));
        assert!(!export_wants_bom(&cx_for("/admin/users/export?bom=0")));
        assert!(!export_wants_bom(&cx_for("/admin/users/export?BOM=1")));
    }

    #[test]
    fn export_cap_maps_one_row_past_the_limit_to_413() {
        // GH #94: the cap branch must produce a content-too-large error, not
        // just a constant that happens to equal 10_000. Exercised at the
        // boundary.
        let under_cap = enforce_export_cap(vec![0u8; MAX_EXPORT_ROWS]).unwrap();
        assert_eq!(under_cap.len(), MAX_EXPORT_ROWS);
        let err = enforce_export_cap(vec![0u8; MAX_EXPORT_ROWS + 1]).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "cap must map to content-too-large (413), got {err}"
        );
    }

    #[test]
    fn export_cap_counts_only_viewable_rows() {
        // GH #145 (with GH #86): visibility is applied before the cap, so a
        // table with many invisible rows exports its visible rows instead of
        // 413ing — the 413 also no longer leaks the invisible-row count.
        let all_denied = filter_then_cap(vec![0u8; MAX_EXPORT_ROWS + 1], |_| false).unwrap();
        assert!(
            all_denied.is_empty(),
            "an all-denied export returns 200 with zero rows, never 413"
        );

        // MAX visible rows plus one denied row fits under the cap.
        let mut rows = vec![1u8; MAX_EXPORT_ROWS];
        rows.push(2u8);
        let capped = filter_then_cap(rows, |r| *r == 1).unwrap();
        assert_eq!(capped.len(), MAX_EXPORT_ROWS);

        // Mixed interleave (bounded over-fetch, GH #145): a full MAX+1
        // window with denied rows inside it exports only the visible ones —
        // visibly fewer than the caller could receive — without 413. This is
        // the issue's accepted alternative ("or document over-fetch"); no
        // truncation signal is emitted because a window-full marker would
        // leak the pre-visibility row count ("no count leak").
        let mixed: Vec<usize> = (0..MAX_EXPORT_ROWS + 1)
            .map(|i| if i % 2 == 0 { i } else { usize::MAX })
            .collect();
        let visible = filter_then_cap(mixed, |r| *r != usize::MAX).unwrap();
        assert_eq!(
            visible.len(),
            (MAX_EXPORT_ROWS + 1).div_ceil(2),
            "mixed window exports its visible rows silently"
        );

        // More visible rows than the cap still 413.
        let err = filter_then_cap(vec![0u8; MAX_EXPORT_ROWS + 1], |_| true).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "cap must map to content-too-large (413), got {err}"
        );
    }

    #[test]
    fn export_filename_cannot_split_the_disposition_header() {
        // GH #145: `slug()` is an overridable free-form String, so quote and
        // control characters must never reach the Content-Disposition header.
        assert_eq!(export_filename("users"), "users.csv");
        for hostile in [
            "a\"b\r\nContent-Length: 0",
            "a\\\"b",
            "\nadmin",
            "bad\u{0}name",
        ] {
            let filename = export_filename(hostile);
            assert!(
                !filename.contains('"')
                    && !filename.contains('\\')
                    && !filename.contains('\r')
                    && !filename.contains('\n')
                    && !filename.chars().any(char::is_control),
                "hostile slug {hostile:?} must be defused, got {filename:?}"
            );
            assert!(filename.ends_with(".csv"), "suffix kept: {filename:?}");
        }
        // A slug that defuses to nothing falls back to a usable filename.
        assert_eq!(export_filename(""), "export.csv");
        assert_eq!(export_filename("\""), "export.csv");
        // Bounded header value.
        let long = "x".repeat(500);
        assert_eq!(export_filename(&long).len(), 100 + ".csv".len());
    }

    #[tokio::test]
    async fn tenant_gated_resource_fails_closed_without_tenant() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct GatedResource;
        impl Resource for GatedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn requires_tenant() -> bool {
                true
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<GatedResource>()
            .auth(crate::Auth::disabled())
            .build();
        // No tenant anywhere → 403, not unscoped rows (GH #87).
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        // Valid CSRF but still no tenant → 403 from the tenant gate.
        let token = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/create")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("name=Ada&csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        // A server-set `Tenant` request extension supplies the tenant → gate
        // passes (create page 200). The header no longer does (GH #131).
        let tenant = uuid::Uuid::new_v4();
        let (mut parts, ()) = http::Request::builder()
            .uri("/admin/dummies/create")
            .body(())
            .unwrap()
            .into_parts();
        parts.extensions.insert(crate::Tenant(tenant));
        let resp = router
            .handle(http::Request::from_parts(parts, Body::empty()))
            .await;
        assert!(
            resp.status().is_success(),
            "tenant-gated GET with tenant must pass the gate, got {}",
            resp.status()
        );
    }

    /// Post/Redirect/Get (GH #97, #126): a mutation answers 303, the flash
    /// cookie rides the error response (Topcoat flushes `Set-Cookie` on `Err`,
    /// topcoat#408), and nothing rides the `Location` query. Following the
    /// redirect consumes the cookie, so a reload does not replay the toast.
    #[tokio::test]
    async fn mutation_redirect_carries_the_flash_cookie_instead_of_a_query() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        const COOKIE_NAME: &str = crate::notification::COOKIE_NAME;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct NotifyingResource;
        impl Resource for NotifyingResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::empty()
            }
            async fn create_record(
                _cx: &Cx,
                _values: HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<NotifyingResource>()
            .auth(crate::Auth::disabled())
            .build();
        let token = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/create")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::SEE_OTHER,
            "a completed mutation is a 303 Post/Redirect/Get"
        );
        let location = resp
            .headers()
            .get(http::header::LOCATION)
            .expect("the redirect names its target")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            !location.contains("notification"),
            "the toast must not ride the query, got {location}"
        );
        let set_cookie = resp
            .headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{COOKIE_NAME}=")))
            .expect("the flash cookie flushes on the Err redirect")
            .to_string();
        assert!(
            set_cookie.contains("success") && set_cookie.contains("Created"),
            "the cookie carries the toast status and title: {set_cookie}"
        );
        assert!(
            set_cookie.contains("Secure") && set_cookie.contains("HttpOnly"),
            "the flushed cookie keeps the __Host- contract: {set_cookie}"
        );
    }

    #[tokio::test]
    async fn layout_shell_renders_a_complete_document() {
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let (parts, ()) = http::Request::builder()
            .uri("/admin/users")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(vec![NavigationItem {
                label: "Users".to_string(),
                url: "/admin/users".to_string(),
                href_check: None,
                order: 0,
            }])
            .build();
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::layout_shell(&cx, slot)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);

        assert!(
            html.starts_with("<!DOCTYPE html>"),
            "missing doctype in {html}"
        );
        assert!(
            html.contains("<html>") && html.contains("<head>"),
            "missing document head in {html}"
        );
        assert!(
            html.contains("<title>Admin</title>"),
            "missing document title in {html}"
        );
        assert!(html.contains("hello"), "missing layout slot in {html}");
    }

    #[tokio::test]
    async fn shell_escapes_brand_name_and_logo() {
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        // Attribute-injection safety rests on `view!` escaping (GH #102):
        // lock it with a hostile brand on both render paths (header + sidebar).
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(
                Brand::new("<script>alert(1)</script>").logo("\"><script>alert(2)</script>"),
            )
            .build();
        let nav_items = vec![NavigationItem {
            label: "Users".to_string(),
            url: "/admin/users".to_string(),
            href_check: None,
            order: 0,
        }];
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &nav_items, "/admin/users", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("\"><script>"),
            "brand must not break out of attributes, got {html}"
        );
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "missing escaped brand text, got {html}"
        );
        assert!(
            html.contains("&quot;"),
            "attribute quotes must be escaped, got {html}"
        );
    }

    #[tokio::test]
    async fn shell_dark_mode_sets_html_class_and_toggle() {
        fn cx_with(dark: Option<bool>) -> Cx {
            use topcoat::context::CxTestBuilder;
            let (parts, ()) = http::Request::builder()
                .uri("/admin/users")
                .body(())
                .unwrap()
                .into_parts();
            match dark {
                Some(v) => CxTestBuilder::new()
                    .request_context(parts)
                    .app_context(DarkMode(v)),
                None => CxTestBuilder::new().request_context(parts),
            }
            .build()
        }

        async fn document_html(cx: &Cx) -> String {
            let slot = view! { cx => "hello" }.boxed().into();
            Panel::layout_shell(cx, slot)
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(cx)
        }

        let html = document_html(&cx_with(Some(true))).await;
        assert!(
            html.contains("<html class=\"dark\">"),
            "DarkMode(true) must set the html class, got {html}"
        );
        assert!(
            html.contains("data-theme-toggle"),
            "theme toggle must render, got {html}"
        );

        let html = document_html(&cx_with(None)).await;
        assert!(
            html.contains("<html>"),
            "no DarkMode must not set the dark class, got {html}"
        );
    }

    #[tokio::test]
    async fn shell_notification_carries_dismiss_hooks() {
        // GH #97/#151: the shell toast is the shadcn/Sonner surface, carrying
        // the auto-dismiss hooks notifications.js arms (mount + 4s + close).
        use crate::notification::Notification;

        let enc = serde_json::to_string(&Notification::success("Created")).unwrap();
        let html = shell_html_with_flash(&enc).await;
        assert!(
            html.contains("data-sonner-toast") && html.contains("data-type=\"success\""),
            "shell toast must be the Sonner surface, got {html}"
        );
        assert!(
            html.contains("data-close-button"),
            "shell toast must carry the Sonner close button, got {html}"
        );
        assert!(
            html.contains("data-title") && html.contains("Created"),
            "shell toast must carry the title, got {html}"
        );
        assert!(
            !html.contains("data-description"),
            "a title-only toast renders no description, got {html}"
        );

        // Error + description: the destructive icon and the supporting line.
        let enc = serde_json::to_string(&Notification::error("Boom").description("What happened"))
            .unwrap();
        let html = shell_html_with_flash(&enc).await;
        assert!(
            html.contains("data-type=\"error\"") && html.contains("text-destructive"),
            "an error toast must carry its type and destructive icon, got {html}"
        );
        assert!(
            html.contains("data-description") && html.contains("What happened"),
            "the description must render, got {html}"
        );
    }

    /// Render the shell once with a flash cookie carrying `enc`.
    async fn shell_html_with_flash(enc: &str) -> String {
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::cookie::CookieJarCell;
        use topcoat::view::view;

        let mut parts = http::Request::builder()
            .uri("/admin/users")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        parts.headers.insert(
            http::header::COOKIE,
            format!("{}={enc}", crate::notification::COOKIE_NAME)
                .parse()
                .unwrap(),
        );
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .request_context(CookieJarCell::new())
            .build();
        let nav_items = vec![NavigationItem {
            label: "Users".to_string(),
            url: "/admin/users".to_string(),
            href_check: None,
            order: 0,
        }];
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        Panel::render_shell(&cx, &nav_items, "/admin/users", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx)
    }

    #[tokio::test]
    async fn sidebar_orders_custom_items_by_sort_key() {
        // GH #102: `.sorted(-1)` interleaves a custom item above the
        // resources; ties keep declaration order.
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let (parts, ()) = http::Request::builder()
            .uri("/admin/users")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        let nav_items = vec![
            NavigationItem {
                label: "Users".to_string(),
                url: "/admin/users".to_string(),
                href_check: None,
                order: 0,
            },
            NavigationItem {
                label: "Showcase".to_string(),
                url: "/admin/showcase".to_string(),
                href_check: None,
                order: 0,
            }
            .sorted(-1),
        ];
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &nav_items, "/admin/users", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let showcase_at = html.find("Showcase").expect("custom item renders");
        let users_at = html.find("Users").expect("resource item renders");
        assert!(
            showcase_at < users_at,
            "sorted(-1) custom item must precede resources, got {html}"
        );
    }

    #[tokio::test]
    async fn panel_shell_renders_sidebar_with_active_and_tokens() {
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let nav_items = vec![
            NavigationItem {
                label: "Users".to_string(),
                url: "/admin/users".to_string(),
                href_check: None,
                order: 0,
            },
            NavigationItem {
                label: "Showcase".to_string(),
                url: "/admin/showcase".to_string(),
                href_check: None,
                order: 0,
            },
        ];
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &nav_items, "/admin/users", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // Sidebar chrome with Token classes — the upstream sidebar palette.
        assert!(
            html.contains("border-border")
                && html.contains("bg-background")
                && html.contains("text-sidebar-foreground"),
            "missing Token border/bg/sidebar tokens in {html}"
        );
        assert!(
            html.contains("data-sidebar=\"sidebar\""),
            "missing sidebar data attr in {html}"
        );
        assert!(
            html.contains("data-sidebar=\"provider\""),
            "missing provider data attr in {html}"
        );
        assert!(
            html.contains("data-sidebar=\"inset\""),
            "missing inset data attr in {html}"
        );
        assert!(
            html.contains("data-sidebar=\"group\"") || html.contains("Navigation"),
            "missing sidebar group in {html}"
        );
        // Shadcn parity: sticky h-svh w-(--sidebar-width) panel
        assert!(
            html.contains("md:sticky") && html.contains("md:top-0"),
            "missing md:sticky md:top-0 in {html}"
        );
        assert!(html.contains("md:h-svh"), "missing md:h-svh in {html}");
        assert!(
            html.contains("w-(--sidebar-width)") || html.contains("--sidebar-width"),
            "missing --sidebar-width var in {html}"
        );
        // Header sticky (the inset's child selector)
        assert!(
            html.contains("sticky") && html.contains("top-0"),
            "missing sticky top-0 in {html}"
        );
        // Data-state for collapsible, seeded by the signal (cookie default:
        // expanded) and bound for the browser runtime.
        assert!(
            html.contains("data-state=\"expanded\"")
                && html.contains("data-topcoat-bind:data-state"),
            "missing bound data-state in {html}"
        );
        // No separator: the dead "Resources" placeholder group it divided
        // is gone (GH #102), and a trailing rule with no following group is
        // chrome noise.
        assert!(
            !html.contains("Managed via Resource::query seam"),
            "dead placeholder must be gone, got {html}"
        );
        // The desktop/mobile trigger pair, wired to the runtime signals.
        assert_eq!(
            html.matches("data-sidebar=\"trigger\"").count(),
            2,
            "expected the desktop + mobile trigger pair in {html}"
        );
        assert!(
            html.contains("data-topcoat-on:click"),
            "missing runtime click bindings in {html}"
        );
        assert!(
            html.contains("max-md:hidden") && html.contains("md:hidden"),
            "missing responsive trigger pair in {html}"
        );
        // Active highlight + real navigation links (the href prop, not attrs)
        assert!(
            html.contains("data-active=\"true\"") && html.contains("aria-current=\"page\""),
            "missing active highlight in {html}"
        );
        assert!(
            html.contains("<a") && html.contains("href=\"/admin/users\""),
            "navigation must render as links in {html}"
        );
        // Main container
        assert!(
            html.contains("max-w-7xl") && html.contains("p-6"),
            "missing main max-w-7xl p-6 in {html}"
        );
        // Dark toggle
        assert!(
            html.contains("Toggle dark mode") || html.contains("data-theme-toggle"),
            "missing dark toggle in {html}"
        );
        // Ensure no ac-* remains in shell
        assert!(
            !html.contains("ac-sidebar")
                && !html.contains("ac-main")
                && !html.contains("ac-nav-item"),
            "ac-* should not remain in shell, got {html}"
        );
    }

    #[tokio::test]
    async fn collapsed_sidebar_cookie_seeds_the_signal() {
        // The persisted `sidebar_state` cookie seeds the runtime signal's
        // initial value, so the first paint matches the last choice; from
        // hydration on, the browser owns the state (assets/sidebar.js mirrors
        // it back).
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let (parts, ()) = http::Request::builder()
            .uri("/admin/users")
            .header(http::header::COOKIE, "theme=dark; sidebar_state=collapsed")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        let cx_ref = &cx;
        let nav_items = vec![NavigationItem {
            label: "Users".to_string(),
            url: "/admin/users".to_string(),
            href_check: None,
            order: 0,
        }];
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &nav_items, "/admin/users", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-state=\"collapsed\"")
                && html.contains("data-collapsible=\"offcanvas\""),
            "collapsed cookie must seed the collapsed state in {html}"
        );
    }

    #[tokio::test]
    async fn unique_check_flags_duplicates_for_marked_fields() {
        use crate::schema::{Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Subscriber { email: "a@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        let schema = Schema::new(TextInput::r#for(Subscriber::fields().email()).unique());
        let mut values = HashMap::new();
        values.insert("email".to_string(), "a@b.c".to_string());

        // Create: duplicate → inline error on the field, label-derived.
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &HashMap::new(), &mut ex)
                .await;
        assert_eq!(
            errors.get("email"),
            Some(&vec!["Email has already been taken".to_string()]),
            "duplicate must be flagged, got {errors:?}"
        );

        // Fresh value → no error.
        let mut fresh = HashMap::new();
        fresh.insert("email".to_string(), "other@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &fresh, &HashMap::new(), &mut ex)
                .await;
        assert!(errors.is_empty(), "fresh value must pass, got {errors:?}");

        // Edit: the record's own unchanged value is not a duplicate.
        let mut current = HashMap::new();
        current.insert("email".to_string(), "a@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &current, &mut ex).await;
        assert!(
            errors.is_empty(),
            "own unchanged value must be skipped, got {errors:?}"
        );

        // Edit: changed to someone else's value → flagged again.
        let mut changed_current = HashMap::new();
        changed_current.insert("email".to_string(), "old@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &changed_current, &mut ex)
                .await;
        assert_eq!(
            errors.get("email"),
            Some(&vec!["Email has already been taken".to_string()]),
            "changed-to-duplicate must be flagged, got {errors:?}"
        );

        // Empty values are skipped (GH #88): pair unique() with required() or
        // normalize "" vs NULL, or optional empty duplicates 500 at the driver.
        let mut empty = HashMap::new();
        empty.insert("email".to_string(), "   ".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &empty, &HashMap::new(), &mut ex)
                .await;
        assert!(errors.is_empty(), "empty must be skipped, got {errors:?}");
    }

    #[test]
    fn reject_unknown_form_keys_allows_declared_plus_csrf() {
        use crate::schema::{Schema, TextInput};

        #[derive(Debug, toasty::Model)]
        struct Member {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        let schema = Schema::new(TextInput::r#for(Member::fields().name()));

        // Declared keys + csrf_token pass.
        let values = HashMap::from([
            ("name".to_string(), "Ada".to_string()),
            (
                crate::csrf::FIELD_NAME.to_string(),
                "some-token".to_string(),
            ),
        ]);
        assert!(reject_unknown_form_keys(&schema, &values).is_ok());

        // Absent keys are fine (present-keys-only updates, GH #89).
        let values = HashMap::from([(
            crate::csrf::FIELD_NAME.to_string(),
            "some-token".to_string(),
        )]);
        assert!(reject_unknown_form_keys(&schema, &values).is_ok());

        // role/tenant_id smuggling is a 400.
        let values = HashMap::from([
            ("name".to_string(), "Ada".to_string()),
            ("role".to_string(), "admin".to_string()),
            ("tenant_id".to_string(), "victim".to_string()),
        ]);
        assert!(reject_unknown_form_keys(&schema, &values).is_err());
    }

    #[test]
    fn form_values_decode_utf8_plus_and_encoded_separators() {
        // Multi-byte UTF-8: %C3%A9 must assemble to é (the old hand-rolled
        // decoder pushed each byte through `byte as char` → "Ã©", GH #75
        // item 6).
        let got = form_values_from_bytes(b"name=R%C3%A9mi");
        assert_eq!(got.get("name").map(String::as_str), Some("Rémi"));

        // `+` is a space; a literal plus is %2B — not double-decoded to space.
        let got = form_values_from_bytes(b"q=a+b&p=C%2B%2B");
        assert_eq!(got.get("q").map(String::as_str), Some("a b"));
        assert_eq!(got.get("p").map(String::as_str), Some("C++"));

        // Encoded separators survive as values.
        let got = form_values_from_bytes(b"a=1%262%3D3");
        assert_eq!(got.get("a").map(String::as_str), Some("1&2=3"));

        // Empty / blank input → empty map.
        assert!(form_values_from_bytes(b"").is_empty());
    }

    /// Build a request context carrying `content_type` and run the streaming
    /// multipart parser over `body` (GH #90).
    async fn multipart_values(
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<HashMap<String, String>, topcoat::Error> {
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users/create")
            .header(http::header::CONTENT_TYPE, content_type)
            .body(())
            .unwrap()
            .into_parts();
        let cx = topcoat::context::CxTestBuilder::new()
            .request_context(parts)
            .build();
        parse_multipart_values(&cx, Body::from(body)).await
    }

    fn multipart_type(boundary: &str) -> String {
        format!("multipart/form-data; boundary={boundary}")
    }

    /// The multipart drain's byte accounting 413s one byte past the cap
    /// (GH #149 tripwire). Unit-tested at the boundary because through the
    /// router the extractor's `BodyLimit` classifies the same body first —
    /// the counter is the backstop for the day that limit stops wrapping the
    /// stream, not a competing cap.
    #[test]
    fn multipart_drain_counts_bytes_and_413s_one_past_the_cap() {
        let mut seen = 0usize;
        count_form_bytes(&mut seen, MAX_FORM_BYTES / 2).unwrap();
        count_form_bytes(&mut seen, MAX_FORM_BYTES / 2).unwrap();
        assert_eq!(seen, MAX_FORM_BYTES);
        let err = count_form_bytes(&mut seen, 1).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "one byte past the cap must map to content-too-large (413), got {err}"
        );
        // A single chunk past the cap fires without a prior accumulation.
        let mut seen = 0usize;
        let err = count_form_bytes(&mut seen, MAX_FORM_BYTES + 1).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "a single over-cap chunk must 413, got {err}"
        );
    }

    /// An 11 MiB multipart upload 413s end to end through the panel (GH #149
    /// acceptance). The installed `BodyLimit::max(MAX_FORM_BYTES)` layer and
    /// the drain's own counter share the same threshold, so the body is over
    /// both at once — the e2e pins the streaming path answers 413 rather than
    /// draining; the counter's own accounting (which only answers if the
    /// extractor's limit ever stops wrapping the stream — `BodyLimitKind` is
    /// private, so no public configuration can disable it) is pinned by
    /// [`count_form_bytes`]'s unit test above.
    #[tokio::test]
    async fn multipart_over_the_form_cap_413s_through_the_router() {
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
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(crate::schema::FileUpload::r#for(Dummy::fields().name()))
            }
            async fn create_record(
                _cx: &Cx,
                _values: std::collections::HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> topcoat::Result<()> {
                Ok(())
            }
        }

        let db = Db::builder().connect("sqlite::memory:").await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<DummyResource>()
            .auth(crate::Auth::disabled())
            .build();
        let boundary = "----Boundary123";
        let payload = "x".repeat(MAX_FORM_BYTES + 1024);
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"; filename=\"big.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n{payload}\r\n--{boundary}--\r\n"
        );
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("/admin/dummies/create")
            .header(
                http::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let resp = router.handle(request).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "an 11 MiB multipart upload must 413 through the router, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn multipart_stream_stores_text_and_filenames() {
        let boundary = "----Boundary123";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHello\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nBINARYBYTES\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nrust,async\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nsecond-wins\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let got = multipart_values(&multipart_type(boundary), body.into_bytes())
            .await
            .unwrap();
        assert_eq!(got.get("title").map(String::as_str), Some("Hello"));
        // v1 stores the filename, not the bytes (FileUpload contract).
        assert_eq!(got.get("image_path").map(String::as_str), Some("photo.jpg"));
        assert_eq!(got.get("tags").map(String::as_str), Some("second-wins"));

        // Empty filename → empty value so `required` fires.
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n--{b}--\r\n",
            b = boundary
        );
        let got = multipart_values(&multipart_type(boundary), body.into_bytes())
            .await
            .unwrap();
        assert_eq!(got.get("image_path").map(String::as_str), Some(""));
    }

    #[tokio::test]
    async fn multipart_stream_sanitizes_traversal_and_filename_star() {
        // Traversal filename lands sanitized (GH #90).
        let body = "--B\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"../../../etc/passwd\"\r\nContent-Type: application/octet-stream\r\n\r\nBYTES\r\n--B--\r\n";
        let got = multipart_values(&multipart_type("B"), body.as_bytes().to_vec())
            .await
            .unwrap();
        assert_eq!(got.get("image_path").map(String::as_str), Some("passwd"));

        // RFC 5987 filename* decodes and wins over filename= (both present).
        let body = "--B\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"plain.jpg\"; filename*=UTF-8''%E2%82%ACphoto.jpg\r\nContent-Type: image/jpeg\r\n\r\nBYTES\r\n--B--\r\n";
        let got = multipart_values(&multipart_type("B"), body.as_bytes().to_vec())
            .await
            .unwrap();
        assert_eq!(
            got.get("image_path").map(String::as_str),
            Some("€photo.jpg"),
            "filename*=UTF-8 must decode and win, got {got:?}"
        );
        // Non-UTF-8 charset falls back to plain filename=.
        let body = "--B\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"plain.jpg\"; filename*=latin-1''%E9.jpg\r\nContent-Type: image/jpeg\r\n\r\nBYTES\r\n--B--\r\n";
        let got = multipart_values(&multipart_type("B"), body.as_bytes().to_vec())
            .await
            .unwrap();
        assert_eq!(
            got.get("image_path").map(String::as_str),
            Some("plain.jpg"),
            "unsupported charset must fall back, got {got:?}"
        );
    }

    #[tokio::test]
    async fn multipart_stream_rejects_missing_boundary() {
        // Bare multipart without boundary is a 400, not a silent urlencoded
        // fallback that turns binary bytes into confusing required-errors.
        assert!(
            multipart_values("multipart/form-data", b"name=x".to_vec())
                .await
                .is_err()
        );
    }

    #[test]
    fn filenames_sanitize_to_basename_and_dispatch_guards_size() {
        assert_eq!(sanitize_filename("upload.jpg"), "upload.jpg");
        assert_eq!(sanitize_filename("../../../etc/cron.d/x"), "x");
        assert_eq!(sanitize_filename("/abs/path"), "path");
        assert_eq!(sanitize_filename("C:\\fakepath\\x"), "x");
        assert_eq!(sanitize_filename(""), "");
        // Names that could never be a safe persisted file are rejected to
        // empty (GH #149): dot/dot-dot, and Windows reserved device names —
        // case-insensitively and with an extension too.
        assert_eq!(sanitize_filename("."), "");
        assert_eq!(sanitize_filename(".."), "");
        assert_eq!(sanitize_filename("../.."), "");
        assert_eq!(sanitize_filename("..."), "...");
        assert_eq!(sanitize_filename("con"), "");
        assert_eq!(sanitize_filename("NUL"), "");
        assert_eq!(sanitize_filename("Com1.txt"), "");
        assert_eq!(sanitize_filename("lpt9"), "");
        assert_eq!(sanitize_filename("console.txt"), "console.txt");
        assert_eq!(sanitize_filename("companion"), "companion");
        assert_eq!(sanitize_filename("...."), "....");
        // Cap keeps the tail without splitting a multibyte char: a naive
        // `[len - 255..]` slice panics here (the cut lands inside `é`).
        let multibyte = format!("{}{}", "é".repeat(200), "a".repeat(200));
        let capped = sanitize_filename(&multibyte);
        assert!(
            capped.len() <= 255,
            "cap must bound bytes, got {}",
            capped.len()
        );
        assert!(
            capped.ends_with('a'),
            "tail must be preserved, got {capped:?}"
        );
        // Over-cap body is rejected before buffering into maps.
        let big = vec![b'a'; MAX_FORM_BYTES + 1];
        assert!(
            form_values_from_request_parts(Some("application/x-www-form-urlencoded"), &big)
                .is_err()
        );
        // Normal urlencoded still parses.
        let ok =
            form_values_from_request_parts(Some("application/x-www-form-urlencoded"), b"name=Ada")
                .unwrap();
        assert_eq!(ok.get("name").map(String::as_str), Some("Ada"));
    }

    #[tokio::test]
    async fn find_by_key_loads_one_row_scoped_and_404s_malformed() {
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let a = toasty::create!(Subscriber { email: "a@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        toasty::create!(Subscriber { email: "z@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        // Existing id → exactly that row (typed PK filter, not a full scan).
        let got = find_by_key::<SubscriberResource>(&cx, &a.id.to_string(), &mut ex)
            .await
            .unwrap();
        assert_eq!(got.id, a.id);

        // Well-formed but unknown id → 404.
        let missing = uuid::Uuid::new_v4().to_string();
        assert!(
            find_by_key::<SubscriberResource>(&cx, &missing, &mut ex)
                .await
                .is_err(),
            "unknown id must not resolve"
        );

        // Malformed id (not a Uuid) → 404, not a query error.
        assert!(
            find_by_key::<SubscriberResource>(&cx, "not-a-uuid", &mut ex)
                .await
                .is_err(),
            "malformed id must not resolve"
        );
    }

    #[tokio::test]
    async fn composite_pk_edit_fails_loudly_not_404() {
        // GH #95: a composite-PK resource is a programming error the URL
        // scheme cannot serve — 500 with a message, never per-id 404s.
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, Clone, toasty::Model)]
        struct Pair {
            #[key]
            a: String,
            #[key]
            b: String,
            name: String,
        }
        struct PairResource;
        impl Resource for PairResource {
            type Model = Pair;
            fn slug() -> String {
                "pairs".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Pair) -> bool {
                true
            }
            fn can_update(_cx: &Cx, _record: &Pair) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Pair> {
                crate::resource::Table::r#for(cx)
                    .id(|p: &Pair| format!("{}-{}", p.a, p.b))
                    .columns(crate::resource::TextColumn::r#for(
                        Pair::fields().name(),
                        |p: &Pair| p.name.clone(),
                    ))
            }
            fn hydrate_form_values(_record: &Pair) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Pair))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<PairResource>()
            .auth(crate::Auth::disabled())
            .build();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/pairs/whatever/edit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "composite PK must fail loudly, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn streamed_list_renders_error_state_when_load_fails() {
        use topcoat::router::Body;

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;

            fn can_view_any(_cx: &Cx) -> bool {
                true
            }

            fn table(_cx: &Cx) -> Table<Self::Model> {
                // A realistic paginated table: the tampered cursor must reach
                // the decode inside `load_table_page` (only paginated loads
                // decode cursors), not die earlier on missing declarations.
                Table::<Subscriber>::new()
                    .id(|s| s.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Subscriber::fields().email(),
                        |s: &Subscriber| s.email.clone(),
                    ))
                    .paginate(25)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Subscriber { email: "a@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<SubscriberResource>()
            .auth(crate::Auth::disabled())
            .build();

        // A tampered `?after=` cursor fails to decode inside the streamed
        // region (GH #79): the page has already streamed with status 200, so
        // the failure must render the branded ErrorState in place — not
        // truncate the stream. This test binary declares no `#[layout]`, so
        // the body is the page fragment stream: page header and toolbar are
        // the "shell still stands" evidence, and the swap payload must be
        // complete (the document-level wrap is proven by the layout tests).
        let response = router
            .handle(
                http::Request::builder()
                    .uri("/admin/subscribers?after=zz-not-a-cursor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            response.status().is_success(),
            "page still streams, got status {}",
            response.status()
        );
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            body.contains(">Subscribers</h1>"),
            "page header must survive the failure: {body}"
        );
        assert!(
            body.contains("Email"),
            "column header must survive the failure: {body}"
        );
        assert!(
            body.contains("Couldn't load Subscribers"),
            "error state must render in the streamed region: {body}"
        );
        // The swap payload arrives complete: topcoat streams swap templates
        // plus the swap script, and a truncated body would cut both.
        assert!(
            body.contains("</template><script>topcoat.swap"),
            "error-state swap payload must be complete: {}",
            &body[body.len().saturating_sub(300)..]
        );
        assert!(
            !body.contains("No records yet"),
            "a failed load is not an empty state: {body}"
        );
        // GH #110: the tampered cursor is the failure itself, so the retry link
        // drops `after`/`before` instead of re-requesting the identical broken
        // URL forever. The rest of the list state still retries.
        assert!(
            body.contains("href=\"/admin/subscribers\""),
            "retry link must target the bare list (cursor dropped): {body}"
        );
        assert!(
            !body.contains("after="),
            "a malformed cursor must not travel into the retry link: {body}"
        );
    }

    #[test]
    fn retry_url_for_error_drops_only_bad_cursors() {
        // GH #110: a malformed cursor can never decode, so its retry link drops
        // pagination; any other failure keeps the full evidence (GH #98).
        let state = TableState {
            search: Some("Ada".to_string()),
            after: Some("cur".to_string()),
            ..TableState::default()
        };
        let bad_cursor = crate::cursor::decode("zz").expect_err("malformed cursor must fail");
        let retry = retry_url_for_error(&state, &bad_cursor, "/admin/users");
        assert!(
            !retry.contains("after="),
            "bad-cursor retry must drop pagination, got {retry}"
        );
        assert!(
            retry.contains("q=Ada"),
            "bad-cursor retry keeps the other state, got {retry}"
        );

        let db_error = topcoat::Error::from(std::io::Error::other("db unavailable"));
        let retry = retry_url_for_error(&state, &db_error, "/admin/users");
        assert!(
            retry.contains("after=cur"),
            "transient failures retry the same evidence, got {retry}"
        );
    }

    #[tokio::test]
    async fn create_form_uses_multipart_only_with_file_upload() {
        use crate::schema::{FileUpload, Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
            title: String,
        }
        struct WithFile;
        impl Resource for WithFile {
            type Model = Doc;
            fn form(_cx: &Cx) -> Schema {
                Schema::new(FileUpload::r#for(Doc::fields().path()))
            }
        }
        struct WithoutFile;
        impl Resource for WithoutFile {
            type Model = Doc;
            fn form(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Doc::fields().title()))
            }
        }

        async fn html_for<R: Resource>(uri: &str) -> String {
            let (parts, ()) = http::Request::builder()
                .uri(uri)
                .body(())
                .unwrap()
                .into_parts();
            let cx = CxTestBuilder::new().request_context(parts).build();
            render_form_page::<R>(
                &cx,
                format!("Create {}", R::navigation_label()),
                "Create",
                &HashMap::new(),
                &HashMap::new(),
            )
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx)
        }

        let with = html_for::<WithFile>("/admin/docs/create").await;
        assert!(
            with.contains("enctype=\"multipart/form-data\""),
            "file form must be multipart, got {with}"
        );
        let without = html_for::<WithoutFile>("/admin/docs/create").await;
        assert!(
            !without.contains("multipart/form-data"),
            "plain form must stay urlencoded, got {without}"
        );
    }
}
