//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`, and registers each
//! declared [`Resource`]'s list page at `{prefix}/{slug}` (Filament-style
//! routes — ADR-0008). See `CONTEXT.md`.

use toasty::Db;
use topcoat::{
    Result,
    asset::{Asset, AssetConfig, RouterBuilderAssetExt},
    context::{Cx, app_context},
    font::Font,
    router::{Body, PageFn, Router, RouterBuilderDiscoverExt, ViewFuture, error::redirect},
    view::{View, attributes, view},
};

use crate::resource::{NavigationItem, Resource, TablePage, TableState};

use crate::db::db;
use topcoat::router::Path;

/// The admin application.
///
/// ```ignore
/// Panel::new("admin")
///     .app_context(db)
///     .resource::<UserResource>()
///     .build()
/// ```
#[derive(Debug)]
pub struct Panel {
    prefix: String,
    db: Option<Db>,
    assets: Option<AssetConfig>,
    shell_assets: Option<ShellAssets>,
    nav_items: Vec<NavigationItem>,
    pages: Vec<PageFn>,
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
            nav_items: Vec::new(),
            pages: Vec::new(),
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
            nav_items,
            pages,
            root_target,
        } = self;
        let db = db.expect("Panel::build requires a Db via app_context");
        let mut builder = Router::builder().discover().app_context(db);
        if !nav_items.is_empty() {
            builder = builder.app_context(nav_items);
        }
        if let Some(assets) = assets {
            builder = builder.assets(assets);
        }
        if let Some(shell_assets) = shell_assets {
            builder = builder.app_context(shell_assets);
        }
        for page in pages {
            builder = builder.page(page);
        }
        // Filament's panel root is a Dashboard; until dashboards exist
        // (GH #38), the prefix serves a redirect to the first resource's
        // list so the mount point is never a dead URL.
        if let Some(target) = root_target {
            builder = builder.app_context(RootRedirect(target)).page(PageFn::new(
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

    async fn theme_toggle(cx: &Cx) -> Result<View> {
        use argentum_ui::{ButtonSize, ButtonVariant, button};

        view! {
            cx =>
            button(
                variant: ButtonVariant::Ghost,
                size: ButtonSize::Icon,
                attrs: attributes! { aria-label="Toggle dark mode" data-theme-toggle="" },
                <span aria-hidden="true">"◐"</span>
            )
        }
    }

    async fn brand(cx: &Cx) -> Result<View> {
        view! {
            cx =>
            <div class="flex items-center gap-2 font-semibold text-foreground">
                "Argentum"
            </div>
        }
    }

    async fn sidebar_navigation(cx: &Cx, menu_items: Vec<View>) -> Result<View> {
        use argentum_ui::{
            sidebar_group, sidebar_group_content, sidebar_group_label, sidebar_menu,
            sidebar_separator,
        };

        view! {
            cx =>
            sidebar_group(
                sidebar_group_label("Navigation")
                sidebar_group_content(
                    sidebar_menu(
                        for item in menu_items {
                            (item)
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
    }

    /// Render the Filament-grade Shell that frames every admin page.
    ///
    /// Composes `argentum-ui` `sidebar` + main `max-w-7xl p-6` area with
    /// grouped NavigationItems, active highlight (`bg-foreground/5` + `aria-current="page"`),
    /// `separator`, and `sidebar_trigger` for responsive `sheet` drawer.
    /// Includes dark-mode toggle (Ghost button, persists via cookie/session) and
    /// notification stack (fixed top-right). Additive `class` is allowed on the
    /// outer container only (narrow seam).
    pub async fn render_shell(
        cx: &Cx,
        nav_items: Vec<NavigationItem>,
        current_path: &str,
        slot: View,
        extra_class: Option<String>,
    ) -> Result<View> {
        use argentum_ui::{
            SheetSide, sheet, sheet_content, sidebar, sidebar_content, sidebar_footer,
            sidebar_header, sidebar_inset, sidebar_menu_button, sidebar_menu_item,
            sidebar_provider, sidebar_trigger,
        };

        let outer_class = extra_class.clone().unwrap_or_default();
        // Build menu items with active detection — delegates to
        // `NavigationItem::is_current_path` (slash-boundary, exact for
        // "/admin") which mirrors `Href::is_current` for string urls;
        // typed `from_href` items delegate to `Href::is_current` via
        // `is_current(cx)` (handles query/encoding). See ADR-0008 / T28.3.
        let mut menu_items: Vec<View> = Vec::new();
        for item in &nav_items {
            // Prefer typed Href when available, else fallback to path string.
            let is_active = if item.href_check.is_some() {
                item.is_current(cx)
            } else {
                item.is_current_path(current_path)
            };
            let label = item.label.clone();
            let url = item.url.clone();
            let btn = view! {
                cx =>
                sidebar_menu_item(
                    sidebar_menu_button(
                        is_active: is_active,
                        attrs: attributes! { href=(url.clone()) },
                        (label.clone())
                    )
                )
            }?;
            menu_items.push(btn);
        }

        // Build both containers from the same navigation helper. The mobile
        // sheet intentionally has its own rendered View, but not its own nav
        // tree to maintain.
        let desktop_navigation = Self::sidebar_navigation(cx, menu_items.clone()).await?;
        let mobile_navigation = Self::sidebar_navigation(cx, menu_items).await?;
        let sidebar_brand = Self::brand(cx).await?;
        let mobile_brand = Self::brand(cx).await?;
        let sidebar_theme_toggle = Self::theme_toggle(cx).await?;
        let mobile_theme_toggle = Self::theme_toggle(cx).await?;
        let header_theme_toggle = Self::theme_toggle(cx).await?;

        view! {
            cx =>
            sidebar_provider(
                attrs: attributes! { class=(outer_class) },
                // Sidebar — fixed inset-y-0 h-svh w-(--sidebar-width), hidden on mobile
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
                        <div class="font-semibold text-foreground">"Admin"</div>
                        <div class="ml-auto flex items-center gap-2">
                            (header_theme_toggle)
                        </div>
                    </header>
                    <main class="flex-1 mx-auto max-w-7xl w-full p-6">(slot)</main>
                )
                // Notification stack — fixed top-right, survives Boundary swaps
                <div class="fixed top-4 right-4 z-50 flex flex-col gap-2">
                    // Placeholder: notifications render here via Panel shell's top-level Boundary
                </div>
            )
            // Scripts are owned by the document (layout_shell).
        }
    }

    /// Convenience wrapper for `#[layout]` handlers: takes `slot: Result` and
    /// renders the complete HTML document around the shell.
    ///
    /// The stylesheet and font are supplied to the Panel builder with
    /// [`Self::shell_assets`]. A Panel without those values remains renderable
    /// for tests and custom document owners, but does not pretend that a CSS
    /// bundle exists. Errors from the page slot are returned unchanged.
    pub async fn layout_shell(cx: &Cx, slot: Result) -> Result {
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
        let inner = slot?;
        let shell = Self::render_shell(cx, nav_items, &current, inner, None).await?;
        let head = match try_app_context::<ShellAssets>(cx).copied() {
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
            }?,
            None => view! {
                cx =>
                topcoat::dev::script()
                argentum_ui::theme_init_script()
            }?,
        };
        view! {
            cx =>
            <!DOCTYPE html>
            <html>
                <head>
                    <title>"Admin"</title>
                    (head)
                </head>
                <body>(shell)</body>
            </html>
        }
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
/// (No `#[memoize]` yet: one load per request. It earns its keep once Table
/// renders behind Boundaries — the reactivity slice of GH #13.)
fn resource_list<R: Resource>(cx: &Cx, _body: Body) -> ViewFuture<'_> {
    Box::pin(async move {
        let state = TableState::from_cx(cx);
        let table = R::table(cx);
        let mut query = R::query(cx);
        if let Some(term) = &state.search
            && let Some(expr) = table.search_expr(term)
        {
            query = query.filter(expr);
        }
        for ord in table.order_bys_for_state(&state) {
            query = query.order_by(ord);
        }
        let mut db = db(cx);
        let page = match table.page_size() {
            Some(per_page) => {
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
                TablePage::from_toasty_page(loaded)?
            }
            None => {
                let rows: Vec<R::Model> =
                    query.exec(&mut db).await.map_err(topcoat::Error::from)?;
                rows.into()
            }
        };
        let table_view = table.render(cx, &page).await?;
        let title = R::navigation_label();
        view! { cx =>
            argentum_ui::page(
                argentum_ui::page_header(argentum_ui::page_title((title)))
                argentum_ui::page_content((table_view))
            )
        }
    })
}

/// The panel root: a temporary redirect to the first declared resource's
/// list, so the mount point is never a dead URL (until Dashboards exist,
/// GH #38). Filament registers a Dashboard page here.
fn panel_root_redirect(cx: &Cx, _body: Body) -> ViewFuture<'_> {
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
        let slot = view! { cx_ref => "hello" }.unwrap();
        let html = Panel::layout_shell(&cx, Ok(slot))
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
        let slot = view! { cx_ref => "hello" }.unwrap();
        let html = Panel::render_shell(&cx, nav_items, "/admin/users", slot, None)
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
}
