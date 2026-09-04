//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`, and registers each
//! declared [`Resource`]'s list page at `{prefix}/{slug}` (Filament-style
//! routes — ADR-0008). See `CONTEXT.md`.

use std::collections::HashMap;

use toasty::Db;
use topcoat::view::internal::ThenView;
use topcoat::{
    Result,
    asset::{Asset, AssetConfig, RouterBuilderAssetExt},
    context::{Cx, app_context},
    cookie::RouterBuilderCookieExt,
    font::Font,
    router::{
        Body, PageFn, RouteFn, RouteFuture, Router, RouterBuilderDiscoverExt, Slot,
        error::{forbidden, redirect},
        request::{Bytes, FromRequest},
    },
    view::{BoxView, Child, View, ViewExt, attributes, suspense, view},
};

use crate::db::db;
use crate::notification::{Notification, set_notification, take_notification};
use crate::resource::{NavigationItem, Resource, Table, TablePage, TableState};
use topcoat::context::memoize;
use topcoat::router::Path;
use topcoat::runtime::{Event, shard};

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
    /// Create a brand with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            logo: None,
        }
    }

    /// Attach a logo URL.
    pub fn logo(mut self, logo: impl Into<String>) -> Self {
        self.logo = Some(logo.into());
        self
    }
}

/// Whether the shell starts in dark mode. Persisted via `theme.js` (`localStorage` + `theme` cookie).
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
    pub fn resource<R: Resource>(mut self) -> Self {
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

    /// Build the [`Router`], discovering all `#[page]` / `#[layout]` / `#[shard]`
    /// items linked into the binary, installing the `Db` and the panel
    /// navigation on the `app_context`, registering each declared resource's
    /// list page, and pointing the panel root at the first resource.
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
        } = self;
        let db = db.expect("Panel::build requires a Db via app_context");
        let mut builder = Router::builder().discover().cookies().app_context(db);
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
        // Filament's panel root is a Dashboard; until dashboards exist
        // (GH #38), the prefix serves a redirect to the first resource's
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

    /// Deprecated alias for [`Self::nav_item`].
    #[deprecated(note = "use Panel::nav_item instead")]
    pub fn navigation_item<R: Resource>(&self) -> NavigationItem {
        self.nav_item::<R>()
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

    async fn render_brand(cx: &Cx) -> Result<BoxView<'_>> {
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
                    <img src=(logo_url) alt=(alt) class="h-6 w-6 rounded">
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
    ) -> Result<BoxView<'a>> {
        use argentum_ui::{
            sidebar_group, sidebar_group_content, sidebar_group_label, sidebar_menu,
            sidebar_menu_button, sidebar_menu_item, sidebar_separator,
        };
        let nav_items = nav_items.to_vec();
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
                                    is_active: is_active,
                                    attrs: attributes! { href=(item.url.clone()) },
                                    (item.label.clone())
                                )
                            )
                        }
                    )
                )
            )
            sidebar_separator()
            sidebar_group(
                sidebar_group_label("Resources")
                sidebar_group_content(
                    <div class="px-2 text-xs text-muted-foreground">
                        "Managed via Resource::query seam"
                    </div>
                )
            )
        }
        .boxed())
    }

    /// Render the Filament-grade Shell that frames every admin page.
    ///
    /// Composes `argentum-ui` `sidebar` + main `max-w-7xl p-6` area with
    /// grouped NavigationItems, active highlight (`bg-sidebar-accent` + `aria-current="page"`),
    /// `separator`, and `sidebar_trigger` for responsive `sheet` drawer.
    /// Includes dark-mode toggle (Ghost button, persists via cookie/session) and
    /// notification stack (fixed top-right). Additive `class` is allowed on the
    /// outer container only (narrow seam).
    pub async fn render_shell<'a>(
        cx: &'a Cx,
        nav_items: &[NavigationItem],
        current_path: &str,
        slot: Child<'a>,
        extra_class: Option<String>,
    ) -> Result<BoxView<'a>> {
        use argentum_ui::{
            SheetSide, sheet, sheet_content, sidebar, sidebar_content, sidebar_footer,
            sidebar_header, sidebar_inset, sidebar_provider, sidebar_trigger,
        };

        let outer_class = extra_class.clone().unwrap_or_default();
        let header_title = topcoat::context::try_app_context::<Brand>(cx)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Admin".to_string());
        // Build both containers from the same navigation helper. The mobile
        // sheet intentionally has its own rendered View, but not its own nav
        // tree to maintain.
        let desktop_navigation = Self::sidebar_navigation(cx, nav_items, current_path).await?;
        let mobile_navigation = Self::sidebar_navigation(cx, nav_items, current_path).await?;
        let sidebar_brand = Self::render_brand(cx).await?;
        let mobile_brand = Self::render_brand(cx).await?;
        let sidebar_theme_toggle = Self::theme_toggle(cx).await?;
        let mobile_theme_toggle = Self::theme_toggle(cx).await?;
        let header_theme_toggle = Self::theme_toggle(cx).await?;
        let notification_view: BoxView<'_> = if let Some(notification) =
            take_notification(cx).or_else(|| notification_from_query(cx))
        {
            let title = notification.title.clone();
            view! {
                cx =>
                <div
                    class="rounded-xl border border-border bg-background shadow-sm p-4"
                >
                    <p class="text-sm font-medium text-foreground">(title)</p>
                </div>
            }
            .boxed()
        } else {
            view! { cx => <span></span> }.boxed()
        };

        Ok(view! {
            cx =>
            sidebar_provider(
                attrs: attributes! { class=(outer_class) },
                // Sidebar — fixed inset-y-0 h-svh w-(--sidebar-width), hidden on mobile
                // `sidebar_rail` is intentionally not rendered here: the footer
                // `sidebar_trigger` (small ghost icon) is the explicit toggle.
                // The rail is an edge hit-area (`w-4` with `hover:after:bg-sidebar-border`)
                // per shadcn `sidebar.tsx:282` — it looks like a big vertical line
                // on hover and is confusing as a primary toggle. Keep the
                // component available (`argentum_ui::sidebar_rail`) for opt-in
                // `variant=inset`/`floating` layouts, but don't render it by default.
                sidebar(
                    sidebar_header((sidebar_brand))
                    sidebar_content((desktop_navigation))
                    sidebar_footer(
                        <div class="flex items-center gap-2">
                            sidebar_trigger()
                            (sidebar_theme_toggle)
                        </div>
                    )
                )
                // Mobile Sheet drawer — hidden on lg, w-(--sidebar-width-mobile) when open
                sheet(
                    open: false,
                    attrs: attributes! { id="mobile-sidebar-sheet" class="lg:hidden" },
                    sheet_content(
                        side: SheetSide::Left,
                        attrs: attributes! {
                            class="w-(--sidebar-width-mobile) p-0"
                            data-sidebar="sidebar"
                            data-mobile="true"
                        },
                        sidebar_header((mobile_brand))
                        sidebar_content((mobile_navigation))
                        sidebar_footer(
                            <div class="flex items-center gap-2">
                                sidebar_trigger()
                                (mobile_theme_toggle)
                            </div>
                        )
                    )
                )
                // Gap for fixed sidebar — hidden on mobile, w-(--sidebar-width) on lg, collapses to 0 when offcanvas
                <div
                    class="hidden w-(--sidebar-width) shrink-0 transition-[width] duration-200 group-data-[collapsible=offcanvas]:w-0 lg:block"
                    aria-hidden="true"
                ></div>
                sidebar_inset(
                    // Main content area — sticky header per shadcn. The trigger is
                    // always visible (shadcn SidebarTrigger): below lg it opens the
                    // mobile Sheet, on lg it collapses the rail. It must carry no
                    // `hidden`/`lg:flex` pair — the Ghost base's `inline-flex` comes
                    // after `hidden` in the utilities layer and would win anyway.
                    <header
                        class="sticky top-0 z-10 flex h-16 items-center gap-4 border-b border-border bg-background px-6"
                    >
                        sidebar_trigger(attrs: attributes! { class="-ml-1" })
                        <div class="font-semibold text-foreground">(header_title)</div>
                        <div class="ml-auto flex items-center gap-2">
                            (header_theme_toggle)
                        </div>
                    </header>
                    <main class="flex-1 mx-auto max-w-7xl w-full p-6">(slot)</main>
                )
                // Notification stack — fixed top-right, survives Boundary swaps
                <div class="fixed top-4 right-4 z-50 flex flex-col gap-2">
                    (notification_view)
                </div>
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
                }]
            });
        let shell = Self::render_shell(cx, &nav_items, &current, slot, None).await?;
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
            }
            .boxed(),
            None => view! {
                cx =>
                topcoat::dev::script()
                argentum_ui::theme_init_script()
            }
            .boxed(),
        };
        let brand_title = try_app_context::<Brand>(cx)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Admin".to_string());
        let html_class =
            try_app_context::<DarkMode>(cx).and_then(|dm| if dm.0 { Some("dark") } else { None });
        Ok(view! {
            cx =>
            <!DOCTYPE html>
            <html class=(html_class)>
                <head>
                    <title>(brand_title)</title>
                    (head)
                </head>
                <body>(shell)</body>
            </html>
        })
    }
}

/// Parse a panel route path, panicking on malformed input — the paths are
/// built from the panel prefix and the resource slug, both validated earlier.
fn route_path(path: &str) -> topcoat::router::PathBuf {
    Path::from_str(path)
        .expect("panel route paths are well-formed")
        .to_owned()
}

/// The list page every declared [`Resource`] gets at `{prefix}/{slug}`.
///
/// One generic handler drives all resources: resolve the [`TableState`] from
/// the URL, scope through `Resource::query` (the tenancy seam, ADR-0002),
/// apply the table's search/sort/pagination declarations, render through
/// `Resource::table`. The page title is the resource's navigation label.
///
/// The page streams: shell, header and the search toolbar go out with the
/// first content, while the row grid loads inside a `suspense` region that
/// swaps in the skeleton → table without any client-side fetching.
fn resource_list<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(ThenView::new(async move {
        if !R::can_view_any(cx) {
            return Err(forbidden().into());
        }
        let state = TableState::from_cx(cx);
        let mut table = R::table(cx);
        // Wire delete prefix so Table rows show Delete buttons.
        let prefix = {
            let path = topcoat::context::try_request_context::<http::request::Parts>(cx)
                .map(|parts| parts.uri.path().to_string())
                .unwrap_or_default();
            // Derive base prefix: current path is list path like /admin/users
            // For create/edit/delete we need base like /admin/users
            // Use path as prefix for delete actions.
            path
        };
        table = table.with_delete(prefix.clone()).with_bulk_delete(true);
        let title = R::navigation_label();

        // First content: the skeleton grid (same markup the eager
        // `defer(true)` path renders), while the rows load below.
        let skeleton = table.render_skeleton(cx).await?;
        let lazy_rows = ThenView::new(async move {
            let page = load_table_page::<R>(cx, &table, &state).await?;
            table.render(cx, page).await
        });

        Ok(view! {
            cx =>
            signal q = String::new();
            argentum_ui::page(
                argentum_ui::page_header(argentum_ui::page_title((title.clone())))
                argentum_ui::page_content(
                    <div class="flex flex-col gap-4">
                        <input
                            :value=$(q.get())
                            @input=$(|e: Event| q.set(e.target.value))
                            placeholder="Live search..."
                            class="w-64 border border-border rounded px-2 py-1"
                        >
                        table_shard(q: $(q.get()))
                        suspense(fallback: skeleton, (lazy_rows.boxed()))
                    </div>
                )
            )
        })
    }))
}

/// Resolve the declared table (search / filters / sort / pagination) against
/// `Resource::query` and execute it — the data-loading half of
/// [`resource_list`], kept separate so the page shell can stream before it.
async fn load_table_page<R: Resource>(
    cx: &Cx,
    table: &Table<R::Model>,
    state: &TableState,
) -> Result<TablePage<R::Model>> {
    let mut query = R::query(cx);
    if let Some(term) = &state.search
        && let Some(expr) = table.search_expr(term)
    {
        query = query.filter(expr);
    }
    if let Some(expr) = table.filter_expr(state) {
        query = query.filter(expr);
    }
    for ord in table.order_bys_for_state(state) {
        query = query.order_by(ord);
    }
    let mut db = db(cx);
    match table.page_size() {
        Some(per_page) => {
            // Keep a cursor-free copy of the filtered+ordered query for
            // cursor validation. Toasty's `Page` sets `next_cursor` when
            // `len == page_size` and `prev_cursor` when `has_previous_page`,
            // which leaves phantom cursors when the page sits exactly at a
            // boundary (e.g. a `before` fetch that lands on the first page
            // returns `len == page_size` so `prev_cursor` is set even though
            // `before(prev_cursor)` is empty). Validate such cursors with a
            // cheap `LIMIT 1` probe and hide phantoms.
            let base_query = query.clone();
            let mut paginated = toasty::stmt::Paginate::new(query, per_page);
            if let Some(cursor) = &state.after {
                paginated = paginated.after(crate::cursor::decode(cursor)?);
            } else if let Some(cursor) = &state.before {
                paginated = paginated.before(crate::cursor::decode(cursor)?);
            }
            let loaded = paginated
                .exec(&mut db)
                .await
                .map_err(topcoat::Error::from)?;
            let mut page = TablePage::from_toasty_page(loaded)?;
            // Only probe for phantom cursors when the page is full
            // (`len == per_page`); a short page cannot have a next page
            // and probing would be a wasted round-trip (GH #75).
            if page.rows.len() == per_page {
                if let Some(cursor) = page.next_cursor.clone() {
                    let probe = toasty::stmt::Paginate::new(base_query.clone(), 1)
                        .after(crate::cursor::decode(&cursor)?)
                        .exec(&mut db)
                        .await
                        .map_err(topcoat::Error::from)?;
                    if probe.items.is_empty() {
                        page.next_cursor = None;
                    }
                }
                if let Some(cursor) = page.prev_cursor.clone() {
                    let probe = toasty::stmt::Paginate::new(base_query.clone(), 1)
                        .before(crate::cursor::decode(&cursor)?)
                        .exec(&mut db)
                        .await
                        .map_err(topcoat::Error::from)?;
                    if probe.items.is_empty() {
                        page.prev_cursor = None;
                    }
                }
            } else {
                // Short page → no next, keep prev as-is (has_previous already correct).
                page.next_cursor = None;
            }
            Ok(page)
        }
        None => {
            let rows: Vec<R::Model> = query.exec(&mut db).await.map_err(topcoat::Error::from)?;
            Ok(rows.into())
        }
    }
}

/// Helper: parse `application/x-www-form-urlencoded` body into a map.
/// Falls back to empty map on read error or non-utf8.
async fn parse_form_values(cx: &Cx, body: Body) -> HashMap<String, String> {
    let bytes = match Bytes::from_request(cx, body).await {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };
    if bytes.is_empty() {
        return HashMap::new();
    }
    let s = match String::from_utf8(bytes.to_vec()) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let mut map = HashMap::new();
    for pair in s.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k_dec = percent_decode(k);
        let v_dec = percent_decode(v);
        map.insert(k_dec, v_dec);
    }
    map
}

fn percent_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next().unwrap_or('0');
            let lo = chars.next().unwrap_or('0');
            let hex = format!("{hi}{lo}");
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push(hi);
                out.push(lo);
            }
        } else if c == '+' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

#[memoize]
async fn memoized_dummy(cx: &Cx, q: &str) -> String {
    let _ = cx;
    q.to_string()
}

#[shard]
async fn table_shard(cx: &Cx, q: String) -> Result<impl View> {
    let _ = memoized_dummy(cx, &q).await;
    Ok(view! {
        cx =>
        <div data-boundary="table">
            <p>
                "Shard Table for "
                (q)
            </p>
        </div>
    })
}

/// Helper: compute list URL from current request path (e.g. /admin/users/create -> /admin/users).
fn list_url_for_current(cx: &Cx, fallback_slug: &str) -> String {
    let path = topcoat::router::request::uri(cx).path().to_string();
    // Derive prefix from the current path so `Panel::new("backoffice")`
    // doesn't hard-code `/admin` on fallback (GH #75).
    let prefix = path
        .split('/')
        .nth(1)
        .filter(|s| !s.is_empty())
        .unwrap_or("admin");
    if path.ends_with("/create") {
        path.trim_end_matches("/create").to_string()
    } else if path.ends_with("/edit") {
        // /admin/users/{id}/edit -> /admin/users
        // Remove last two segments: /{id}/edit
        let mut segs: Vec<&str> = path.split('/').collect();
        // segs like ["", "admin", "users", "id", "edit"]
        if segs.len() >= 4 {
            segs.truncate(segs.len() - 2);
            let out = segs.join("/");
            if out.is_empty() { "/".to_string() } else { out }
        } else {
            format!("/{}/{}", prefix, fallback_slug)
        }
    } else if path.contains("/delete") || path.contains("/bulk-delete") {
        let mut segs: Vec<&str> = path.split('/').collect();
        // Remove last segment(s) to get back to list
        if segs.last() == Some(&"delete") {
            segs.pop();
            segs.pop(); // id
        } else if segs.last() == Some(&"bulk-delete") {
            segs.pop();
        }
        let out = segs.join("/");
        if out.is_empty() { "/".to_string() } else { out }
    } else {
        // Default to /{prefix}/{slug}
        format!("/{}/{}", prefix, fallback_slug)
    }
}

fn notification_from_query(cx: &Cx) -> Option<Notification> {
    let query = topcoat::context::try_request_context::<http::request::Parts>(cx)
        .and_then(|parts| parts.uri.query().map(|q| q.to_string()))?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == "notification" {
            let title = percent_decode(v);
            // Decode '+' to space already handled in percent_decode, but ensure.
            return Some(Notification::success(title));
        }
    }
    None
}

async fn render_create_page<'a, R: Resource>(
    cx: &'a Cx,
    values: &HashMap<String, String>,
    errors: &HashMap<String, Vec<String>>,
) -> Result<BoxView<'a>> {
    let schema = R::form(cx);
    let form_html = schema.render_with(cx, values, errors).await?;
    let action = topcoat::router::request::uri(cx).path().to_string();
    let title = format!("Create {}", R::navigation_label());
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(argentum_ui::page_title((title.clone())))
            argentum_ui::page_content(
                <form method="post" action=(action) class="flex flex-col gap-4">
                    (form_html)
                    <div class="flex gap-2">
                        argentum_ui::button(
                            variant: argentum_ui::ButtonVariant::Primary,
                            attrs: attributes! { r#type="submit" },
                            "Create"
                        )
                        <a
                            href=(list_url_for_current(cx, &R::slug()))
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

async fn render_edit_page<'a, R: Resource>(
    cx: &'a Cx,
    _id: &str,
    values: &HashMap<String, String>,
    errors: &HashMap<String, Vec<String>>,
) -> Result<BoxView<'a>> {
    let schema = R::form(cx);
    let form_html = schema.render_with(cx, values, errors).await?;
    let action = topcoat::router::request::uri(cx).path().to_string();
    let title = format!("Edit {}", R::navigation_label());
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(argentum_ui::page_title((title.clone())))
            argentum_ui::page_content(
                <form method="post" action=(action) class="flex flex-col gap-4">
                    (form_html)
                    <div class="flex gap-2">
                        argentum_ui::button(
                            variant: argentum_ui::ButtonVariant::Primary,
                            attrs: attributes! { r#type="submit" },
                            "Save"
                        )
                        <a
                            href=(list_url_for_current(cx, &R::slug()))
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
    Box::pin(ThenView::new(async move {
        if !R::can_create(cx) {
            return Err(forbidden().into());
        }
        let html = render_create_page::<R>(cx, &HashMap::new(), &HashMap::new()).await?;
        Ok(html)
    }))
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
async fn check_unique<R: Resource>(
    cx: &Cx,
    schema: &crate::schema::Schema,
    values: &HashMap<String, String>,
    current: &HashMap<String, String>,
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
        let mut db = db(cx);
        let rows = R::query(cx)
            .filter(input.eq_filter::<R::Model>(submitted))
            .limit(1)
            .exec(&mut db)
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
    Box::pin(ThenView::new(async move {
        if !R::can_create(cx) {
            return Err(forbidden().into());
        }
        let values = parse_form_values(cx, body).await;
        let schema = R::form(cx);
        let mut errors = schema.validate_async(cx, &values).await;
        // App-side unique check over every `unique()`-marked input — the only
        // error layer until toasty exposes a unique-violation predicate
        // (EXTERNAL_GAPS.md; never string-match driver error messages).
        for (name, errs) in check_unique::<R>(cx, &schema, &values, &HashMap::new()).await {
            errors.entry(name).or_default().extend(errs);
        }
        if !errors.is_empty() {
            let html = render_create_page::<R>(cx, &values, &errors).await?;
            return Ok(html);
        }
        // Attempt creation via Resource hook (transaction inside).
        match R::create_record(cx, values.clone()).await {
            Ok(()) => {
                let base = list_url_for_current(cx, &R::slug());
                let list_url = format!("{base}?notification=Created");
                // Also set cookie for Boundary survival (if layer present)
                set_notification(cx, Notification::success("Created"));
                Err(redirect(list_url).into())
            }
            // A unique violation that slipped past the app-side check (a
            // concurrent insert) surfaces as an error, not a string-matched
            // inline message (EXTERNAL_GAPS.md unique-violation entry).
            Err(e) => Err(e),
        }
    }))
}

/// Edit page GET — hydrates form from model via Resource::query seam.
fn resource_edit<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(ThenView::new(async move {
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        // Load via query seam and find by id string via Table row key.
        let mut db = db(cx);
        let candidates = R::query(cx)
            .exec(&mut db)
            .await
            .map_err(topcoat::Error::from)?;
        let table = R::table(cx);
        let record = candidates
            .into_iter()
            .find(|m| table.key_for(m).as_deref() == Some(id.as_str()))
            .ok_or_else(topcoat::router::error::not_found)?;
        if !R::can_view(cx, &record) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &record) {
            return Err(forbidden().into());
        }
        let values = R::hydrate_form_values(&record);
        let html = render_edit_page::<R>(cx, &id, &values, &HashMap::new()).await?;
        Ok(html)
    }))
}

/// Edit page POST — validates, checks Policy::update, mutates via Update projection.
fn resource_edit_post<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(ThenView::new(async move {
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        let mut db = db(cx);
        let candidates = R::query(cx)
            .exec(&mut db)
            .await
            .map_err(topcoat::Error::from)?;
        let table = R::table(cx);
        let record = candidates
            .into_iter()
            .find(|m| table.key_for(m).as_deref() == Some(id.as_str()))
            .ok_or_else(topcoat::router::error::not_found)?;
        if !R::can_update(cx, &record) {
            return Err(forbidden().into());
        }
        let values = parse_form_values(cx, body).await;
        let schema = R::form(cx);
        let mut errors = schema.validate_async(cx, &values).await;
        // Unique check excludes this record's own unchanged values.
        let current = R::hydrate_form_values(&record);
        for (name, errs) in check_unique::<R>(cx, &schema, &values, &current).await {
            errors.entry(name).or_default().extend(errs);
        }
        if !errors.is_empty() {
            let html = render_edit_page::<R>(cx, &id, &values, &errors).await?;
            return Ok(html);
        }
        match R::update_record(cx, id.clone(), values.clone()).await {
            Ok(()) => {
                let base = list_url_for_current(cx, &R::slug());
                let list_url = format!("{base}?notification=Updated");
                set_notification(cx, Notification::success("Updated"));
                Err(redirect(list_url).into())
            }
            // A unique violation that slipped past the app-side check (a
            // concurrent update) surfaces as an error, not a string-matched
            // inline message (EXTERNAL_GAPS.md unique-violation entry).
            Err(e) => Err(e),
        }
    }))
}

/// Delete action POST — requires confirmation, runs in transaction, re-checks Policy.
fn resource_delete<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(ThenView::new(async move {
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        let mut db = db(cx);
        let candidates = R::query(cx)
            .exec(&mut db)
            .await
            .map_err(topcoat::Error::from)?;
        let table = R::table(cx);
        let record = candidates
            .into_iter()
            .find(|m| table.key_for(m).as_deref() == Some(id.as_str()))
            .ok_or_else(topcoat::router::error::not_found)?;
        if !R::can_delete(cx, &record) {
            return Err(forbidden().into());
        }
        let values = parse_form_values(cx, body).await;
        let confirmed = values
            .get("confirm")
            .is_some_and(|v| v == "1" || v == "true" || v == "yes");
        if !confirmed {
            // Render confirmation page.
            let html = view! {
                cx =>
                argentum_ui::page(
                    argentum_ui::page_header(argentum_ui::page_title("Confirm delete"))
                    argentum_ui::page_content(
                        <div
                            class="rounded-xl border border-border bg-background p-6 shadow-sm flex flex-col gap-4"
                        >
                            <p class="text-sm text-foreground">
                                "Are you sure you want to delete this record? This action cannot be undone."
                            </p>
                            <form
                                method="post"
                                action=(topcoat::router::request::uri(cx)
                                    .path()
                                    .to_string())
                                class="flex gap-2"
                            >
                                <input type="hidden" name="confirm" value="1">
                                argentum_ui::button(
                                    variant: argentum_ui::ButtonVariant::Primary,
                                    attrs: attributes! { r#type="submit" },
                                    "Confirm"
                                )
                                <a
                                    href=(list_url_for_current(cx, &R::slug()))
                                    class="inline-flex items-center justify-center rounded-md border border-border bg-background px-4 py-2 text-sm"
                                >
                                    "Cancel"
                                </a>
                            </form>
                        </div>
                    )
                )
            };
            return Ok(html);
        }
        // Perform delete via Resource hook (transaction inside).
        R::delete_record(cx, id).await?;
        let base = list_url_for_current(cx, &R::slug());
        let list_url = format!("{base}?notification=Deleted");
        set_notification(cx, Notification::success("Deleted"));
        Err(redirect(list_url).into())
    }))
}

/// Bulk delete POST — ids via `ids` form field (comma-separated).
fn resource_bulk_delete<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(ThenView::<_, BoxView<'_>>::new(async move {
        let values = parse_form_values(cx, body).await;
        let ids_raw = values.get("ids").cloned().unwrap_or_default();
        let ids: Vec<String> = ids_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return Err(topcoat::router::error::bad_request("no ids provided").into());
        }
        // Load candidates via query and check each id exists and passes policy.
        let mut db = db(cx);
        let candidates = R::query(cx)
            .exec(&mut db)
            .await
            .map_err(topcoat::Error::from)?;
        let table = R::table(cx);
        for id in &ids {
            let rec = candidates
                .iter()
                .find(|m| table.key_for(m).as_deref() == Some(id.as_str()))
                .ok_or_else(topcoat::router::error::not_found)?;
            if !R::can_delete(cx, rec) {
                return Err(forbidden().into());
            }
        }
        // All checks passed — perform bulk delete.
        R::bulk_delete_records(cx, ids).await?;
        let base = list_url_for_current(cx, &R::slug());
        let list_url = format!("{base}?notification=Bulk+deleted");
        set_notification(cx, Notification::success("Bulk deleted"));
        Err(redirect(list_url).into())
    }))
}

/// CSV export — reuses `Resource::query` + `Table` filters/sort, streams `text/csv`.
fn resource_export<R: Resource>(cx: &Cx, _body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        if !R::can_view_any(cx) {
            return Err(forbidden().into());
        }
        let state = TableState::from_cx(cx);
        let table = R::table(cx);
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
        let mut db = db(cx);
        let rows: Vec<R::Model> = query.exec(&mut db).await.map_err(topcoat::Error::from)?;
        // Build TablePage without pagination for CSV (all rows)
        let page: TablePage<R::Model> = rows.into();
        let csv = table.to_csv(&page);
        let filename = format!("{}.csv", R::slug());
        let res = http::Response::builder()
            .status(200)
            .header(http::header::CONTENT_TYPE, "text/csv")
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

    #[tokio::test]
    async fn panel_builds_router_with_db() {
        let db = Db::builder().connect("sqlite::memory:").await.unwrap();
        let router = Panel::new("admin").app_context(db).build();
        // Router built without panic — the real serving test lives in the
        // admin example's integration test.
        drop(router);
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
            },
            NavigationItem {
                label: "Showcase".to_string(),
                url: "/admin/showcase".to_string(),
                href_check: None,
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
        // Sidebar chrome with Token classes
        assert!(
            html.contains("border-border") && html.contains("bg-background"),
            "missing Token border/bg in {html}"
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
        // Shadcn parity: fixed + h-svh + w-(--sidebar-width) + gap
        assert!(
            html.contains("fixed") && html.contains("inset-y-0"),
            "missing fixed inset-y-0 in {html}"
        );
        assert!(html.contains("h-svh"), "missing h-svh in {html}");
        assert!(
            html.contains("w-(--sidebar-width)") || html.contains("--sidebar-width"),
            "missing --sidebar-width var in {html}"
        );
        // Header sticky
        assert!(
            html.contains("sticky") && html.contains("top-0"),
            "missing sticky top-0 in {html}"
        );
        // Data-state for collapsible
        assert!(
            html.contains("data-state=\"expanded\"") || html.contains("data-state"),
            "missing data-state in {html}"
        );
        // Separator
        assert!(
            html.contains("shrink-0") && html.contains("bg-border"),
            "missing separator in {html}"
        );
        // Sheet trigger hook
        assert!(
            html.contains("data-sidebar=\"trigger\""),
            "missing sidebar_trigger hook in {html}"
        );
        // Active highlight
        assert!(
            html.contains("bg-foreground/5") && html.contains("aria-current=\"page\""),
            "missing active highlight in {html}"
        );
        // Responsive hook
        assert!(
            html.contains("hidden") && html.contains("lg:flex"),
            "missing responsive hidden lg:flex in {html}"
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

        let schema = Schema::new(TextInput::r#for(Subscriber::fields().email()).unique());
        let mut values = HashMap::new();
        values.insert("email".to_string(), "a@b.c".to_string());

        // Create: duplicate → inline error on the field, label-derived.
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &HashMap::new()).await;
        assert_eq!(
            errors.get("email"),
            Some(&vec!["Email has already been taken".to_string()]),
            "duplicate must be flagged, got {errors:?}"
        );

        // Fresh value → no error.
        let mut fresh = HashMap::new();
        fresh.insert("email".to_string(), "other@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &fresh, &HashMap::new()).await;
        assert!(errors.is_empty(), "fresh value must pass, got {errors:?}");

        // Edit: the record's own unchanged value is not a duplicate.
        let mut current = HashMap::new();
        current.insert("email".to_string(), "a@b.c".to_string());
        let errors = check_unique::<SubscriberResource>(&cx, &schema, &values, &current).await;
        assert!(
            errors.is_empty(),
            "own unchanged value must be skipped, got {errors:?}"
        );

        // Edit: changed to someone else's value → flagged again.
        let mut changed_current = HashMap::new();
        changed_current.insert("email".to_string(), "old@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &changed_current).await;
        assert_eq!(
            errors.get("email"),
            Some(&vec!["Email has already been taken".to_string()]),
            "changed-to-duplicate must be flagged, got {errors:?}"
        );
    }
}
