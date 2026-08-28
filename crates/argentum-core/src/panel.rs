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
        Self { prefix, db: None }
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
    pub fn resource<R: Resource>(self) -> Self {
        let _ = std::any::type_name::<R>();
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
        Router::builder().discover().app_context(db).build()
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
            ButtonSize, ButtonVariant, button, sidebar, sidebar_content, sidebar_footer,
            sidebar_group, sidebar_group_content, sidebar_group_label, sidebar_header,
            sidebar_menu, sidebar_menu_button, sidebar_menu_item, sidebar_separator,
            sidebar_trigger,
        };

        let outer_class = if let Some(extra) = extra_class {
            format!("flex min-h-screen bg-background {extra}")
        } else {
            "flex min-h-screen bg-background".to_string()
        };

        // Build menu items with active detection — delegates to
        // `NavigationItem::is_current_path` which mirrors `Href::is_current`
        // (slash-boundary, exact for "/admin", ignores query). See ADR-0008 / T5.
        let mut menu_items: Vec<View> = Vec::new();
        for item in &nav_items {
            let is_active = item.is_current_path(current_path);
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

        view! {
            cx =>
            <div class=(outer_class)>
                // Sidebar — persistent rail on desktop, hidden on mobile
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
                            sidebar_trigger(attrs: attributes! { class="lg:hidden" })
                            button(
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::Icon,
                                attrs: attributes! { aria-label="Toggle dark mode" data-theme-toggle="" },
                                <span aria-hidden="true">"◐"</span>
                            )
                        </div>
                    )
                )
                // Mobile sheet drawer placeholder — hidden trigger opens sheet
                <div class="lg:hidden">
                    sidebar_trigger(attrs: attributes! { class="m-2" })
                </div>
                // Main content area
                <div class="flex flex-1 flex-col min-w-0">
                    <header class="flex h-16 items-center gap-4 border-b border-border bg-background px-6">
                        sidebar_trigger(attrs: attributes! { class="lg:hidden" })
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
                </div>
                // Notification stack — fixed top-right, survives Boundary swaps
                <div class="fixed top-4 right-4 z-50 flex flex-col gap-2">
                    // Placeholder: notifications render here via Panel shell's top-level Boundary
                </div>
            </div>
        }
    }

    /// Convenience wrapper for `#[layout]` handlers: takes `slot: Result` and
    /// renders the shell with current path derived from `cx`.
    pub async fn layout_shell(cx: &Cx, slot: Result) -> Result {
        use topcoat::router::request::uri;
        let current = uri(cx).path().to_string();
        // Default nav — single resource placeholder; real apps pass explicit nav_items
        // via `render_shell`. This fallback keeps empty projects beautiful out of the box
        // with at least one NavigationItem.
        let nav_items = vec![NavigationItem {
            label: "Dashboard".to_string(),
            url: "/admin".to_string(),
        }];
        let inner = slot?;
        Self::render_shell(cx, nav_items, &current, inner, None).await
    }
}

/// Default `#[layout("/admin")]` — provides zero-boilerplate shell for
/// `Panel::new("admin").resource::<R>().build()` (ADR-0008). Discovered via
/// `Router::builder().discover()`. Apps needing custom nav define their own
/// `#[layout("/admin")]` which takes precedence or calls `Panel::render_shell`
/// directly.
#[topcoat::router::layout("/admin")]
async fn argentum_shell(cx: &Cx, slot: Result) -> Result {
    Panel::layout_shell(cx, slot).await
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
            NavigationItem {
                label: "Users".to_string(),
                url: "/admin".to_string(),
            },
            NavigationItem {
                label: "Showcase".to_string(),
                url: "/admin/showcase".to_string(),
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
            html.contains("data-sidebar=\"group\"") || html.contains("Navigation"),
            "missing sidebar group in {html}"
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
