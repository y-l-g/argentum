//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`. See `CONTEXT.md`.

use toasty::Db;
use topcoat::{
    Result,
    context::Cx,
    router::{Router, RouterBuilderDiscoverExt},
    view::{View, attributes, view},
};

use crate::resource::{NavigationItem, Resource};

/// The admin application.
///
/// ```ignore
/// Panel::new("admin").app_context(db).build()
/// ```
#[derive(Debug)]
pub struct Panel {
    prefix: String,
    db: Option<Db>,
    nav_items: Vec<NavigationItem>,
}

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
            nav_items: Vec::new(),
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

    /// Declare a `Resource` for this panel (declarative seam).
    ///
    /// `Panel::new("admin").resource::<UserResource>().build()` ensures the
    /// resource's `#[page]`s are linked and documents the panel's closed set
    /// of resources. Multiple calls compose. For now this is a marker — types
    /// are discovered via `Router::builder().discover()` — but it will drive
    /// navigation generation in the full declarative shell (ADR-0008).
    pub fn resource<R: Resource>(mut self) -> Self {
        let item = NavigationItem::from_resource_with_prefix::<R>(&self.prefix);
        self.nav_items.push(item);
        self
    }

    /// Build the [`Router`], discovering all `#[page]` / `#[layout]` / `#[shard]`
    /// items linked into the binary and installing the `Db` on the
    /// `app_context`.
    ///
    /// Panics if no `Db` was provided via [`app_context`](Self::app_context).
    ///
    /// Also validates the Tailwind stylesheet presence and emits a diagnostic
    /// when missing (see `examples/showcase/styles.css` and `build.rs`).
    pub fn build(self) -> Router {
        let db = self.db.expect("Panel::build requires a Db via app_context");
        // Diagnostic for per-app Tailwind contract — see ADR-0006.
        // The stylesheet is generated at `$OUT_DIR/tailwind.css` via
        // `argentum_ui::tailwind_build()` in the app's `build.rs`. In tests
        // or when the build hasn't run, the file is absent; we only warn in
        // non-test builds to keep `cargo test` green.
        #[cfg(not(test))]
        {
            let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
            let css = std::path::Path::new(&out_dir).join("tailwind.css");
            if !css.exists() && std::env::var("ARGENTUM_SKIP_TAILWIND_CHECK").is_err() {
                eprintln!(
                    "warning: Tailwind stylesheet not found at {css:?}. \
                    Ensure `styles.css` (with `@import \"tailwindcss\"` + tokens + `@source` for app and `argentum-ui`) \
                    and `build.rs` (`argentum_ui::tailwind_build()`) are present, and layout injects \
                    `tailwind::stylesheet!()` + `fontsource_font!(GEIST)`. See `examples/showcase` as reference (ADR-0006)."
                );
            }
        }
        let nav_items = self.nav_items;
        Router::builder()
            .discover()
            .app_context(db)
            .app_context(nav_items)
            .build()
    }

    /// Derive a [`NavigationItem`] for `R` using this panel's mount prefix.
    ///
    /// This is the panel-aware counterpart to `NavigationItem::from_resource`.
    /// The URL respects `self.prefix()` so `Panel::new("backoffice")` yields
    /// `"/backoffice"` instead of hard-coded `"/admin"`.
    pub fn navigation_item<R: Resource>(&self) -> NavigationItem {
        NavigationItem::from_resource_with_prefix::<R>(&self.prefix)
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
            ButtonSize, ButtonVariant, SheetSide, button, sheet, sheet_content, sidebar,
            sidebar_content, sidebar_footer, sidebar_group, sidebar_group_content,
            sidebar_group_label, sidebar_header, sidebar_inset, sidebar_menu,
            sidebar_menu_button, sidebar_menu_item, sidebar_provider, sidebar_separator,
            sidebar_trigger,
        };

        let outer_class = extra_class.clone().unwrap_or_default();
        let has_assets =
            topcoat::context::try_app_context::<topcoat::asset::AssetConfig>(cx).is_some();

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
            }
            .unwrap();
            menu_items.push(btn);
        }

        // Build mobile sheet content — same nav as desktop sidebar, for Sheet drawer
        let mobile_menu_items = menu_items.clone();
        view! {
            cx =>
            sidebar_provider(
                attrs: attributes! { class=(outer_class) },
                // Sidebar — fixed inset-y-0 h-svh w-(--sidebar-width), hidden on mobile
                sidebar(
                    sidebar_header(
                        <div class="flex items-center gap-2 font-semibold text-foreground">
                            "Argentum"
                        </div>
                    )
                    sidebar_content(
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
                                <div class="px-2 text-xs text-muted-foreground">"Managed via Resource::query seam"</div>
                            )
                        )
                    )
                    sidebar_footer(
                        <div class="flex items-center gap-2">
                            sidebar_trigger()
                            button(
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::Icon,
                                attrs: attributes! { aria-label="Toggle dark mode" data-theme-toggle="" },
                                <span aria-hidden="true">"◐"</span>
                            )
                        </div>
                    )
                )
                // Mobile Sheet drawer — hidden on lg, w-(--sidebar-width-mobile) when open
                sheet(
                    open: false,
                    attrs: attributes! { id="mobile-sidebar-sheet" class="lg:hidden" },
                    sheet_content(
                        side: SheetSide::Left,
                        attrs: attributes! { class="w-(--sidebar-width-mobile) p-0" data-sidebar="sidebar" data-mobile="true" },
                        sidebar_header(
                            <div class="flex items-center gap-2 font-semibold text-foreground">
                                "Argentum"
                            </div>
                        )
                        sidebar_content(
                            sidebar_group(
                                sidebar_group_label("Navigation")
                                sidebar_group_content(
                                    sidebar_menu(
                                        for item in mobile_menu_items {
                                            (item)
                                        }
                                    )
                                )
                            )
                            sidebar_separator()
                            sidebar_group(
                                sidebar_group_label("Resources")
                                sidebar_group_content(
                                    <div class="px-2 text-xs text-muted-foreground">"Managed via Resource::query seam"</div>
                                )
                            )
                        )
                        sidebar_footer(
                            <div class="flex items-center gap-2">
                                sidebar_trigger()
                                button(
                                    variant: ButtonVariant::Ghost,
                                    size: ButtonSize::Icon,
                                    attrs: attributes! { aria-label="Toggle dark mode" data-theme-toggle="" },
                                    <span aria-hidden="true">"◐"</span>
                                )
                            </div>
                        )
                    )
                )
                // Gap for fixed sidebar — hidden on mobile, w-(--sidebar-width) on lg, collapses to 0 when offcanvas
                <div class="hidden w-(--sidebar-width) shrink-0 transition-[width] duration-200 group-data-[collapsible=offcanvas]:w-0 lg:block" aria-hidden="true"></div>
                sidebar_inset(
                    // Main content area — sticky header per shadcn. The trigger is
                    // always visible (shadcn SidebarTrigger): below lg it opens the
                    // mobile Sheet, on lg it collapses the rail. It must carry no
                    // `hidden`/`lg:flex` pair — the Ghost base's `inline-flex` comes
                    // after `hidden` in the utilities layer and would win anyway.
                    <header class="sticky top-0 z-10 flex h-16 items-center gap-4 border-b border-border bg-background px-6">
                        sidebar_trigger(attrs: attributes! { class="-ml-1" })
                        <div class="font-semibold text-foreground">"Admin"</div>
                        <div class="ml-auto flex items-center gap-2">
                            button(
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::Icon,
                                attrs: attributes! { aria-label="Toggle dark mode" data-theme-toggle="" },
                                <span aria-hidden="true">"◐"</span>
                            )
                        </div>
                    </header>
                    <main class="flex-1 mx-auto max-w-7xl w-full p-6">
                        (slot)
                    </main>
                )
                // Notification stack — fixed top-right, survives Boundary swaps
                <div class="fixed top-4 right-4 z-50 flex flex-col gap-2">
                    // Placeholder: notifications render here via Panel shell's top-level Boundary
                </div>
            )
            if has_assets {
                topcoat::runtime::script()
                <script src=(argentum_ui::SIDEBAR_JS)></script>
                <script src=(argentum_ui::THEME_JS)></script>
            } else {
                // Fallback for tests / offline builds without AssetBundle
                <script>
                    "document.addEventListener('DOMContentLoaded',()=>{const s=document.querySelector('[data-sidebar=\"sidebar\"]');const p=document.querySelector('[data-sidebar=\"provider\"]');const sheet=document.getElementById('mobile-sidebar-sheet');if(s&&p){const setState=c=>{const col=c==='collapsed'?'offcanvas':'';s.setAttribute('data-state',c);s.setAttribute('data-collapsible',col);p.setAttribute('data-state',c);p.setAttribute('data-collapsible',col);document.cookie=`sidebar_state=${c};path=/;max-age=604800`};document.addEventListener('click',e=>{if(e.target.closest('[data-sidebar=\"trigger\"]')){if(window.innerWidth<1024&&sheet){if(sheet.hasAttribute('open')){sheet.removeAttribute('open');sheet.close?.()}else{sheet.setAttribute('open','');sheet.showModal?.()}}else{setState(s.getAttribute('data-state')==='collapsed'?'expanded':'collapsed')}}});document.addEventListener('keydown',e=>{if((e.ctrlKey||e.metaKey)&&e.key==='b'){e.preventDefault();document.querySelector('[data-sidebar=\"trigger\"]')?.click()}});const m=document.cookie.match(/sidebar_state=([^;]+)/);if(m)setState(m[1])}if(sheet){sheet.addEventListener('click',e=>{if(e.target===sheet){sheet.removeAttribute('open');sheet.close?.()}})}document.querySelectorAll('[data-theme-toggle]').forEach(b=>b.addEventListener('click',()=>{document.documentElement.classList.toggle('dark');localStorage.setItem('theme',document.documentElement.classList.contains('dark')?'dark':'light');document.cookie=`theme=${localStorage.getItem('theme')};path=/;max-age=31536000`}));const t=localStorage.getItem('theme')||(document.cookie.match(/theme=([^;]+)/)?.[1]);if(t==='dark')document.documentElement.classList.add('dark'))"
                </script>
            }
        }
    }

    /// Convenience wrapper for `#[layout]` handlers: takes `slot: Result` and
    /// renders the shell with current path derived from `cx`.
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
        Self::render_shell(cx, nav_items, &current, inner, None).await
    }
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
        let item = panel.navigation_item::<DummyResource>();
        assert_eq!(item.label, "Dummys");
        assert_eq!(item.url, "/backoffice");

        let default = Panel::new("admin").navigation_item::<DummyResource>();
        assert_eq!(default.url, "/admin");
        // Also verify the prefix-aware constructor normalises slashes
        let via_prefix = crate::resource::NavigationItem::from_resource_with_prefix::<DummyResource>(
            "/backoffice/",
        );
        assert_eq!(via_prefix.url, "/backoffice");
    }

    #[tokio::test]
    async fn panel_shell_renders_sidebar_with_active_and_tokens() {
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let nav_items = vec![
            NavigationItem {label: "Users".to_string(),
                url: "/admin".to_string(),
            href_check: None,
        },
            NavigationItem {label: "Showcase".to_string(),
                url: "/admin/showcase".to_string(),
            href_check: None,
        },
        ];
        let slot = view! { cx_ref => "hello" }.unwrap();
        let html = Panel::render_shell(&cx, nav_items, "/admin", slot, None)
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
        assert!(
            html.contains("h-svh"),
            "missing h-svh in {html}"
        );
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
