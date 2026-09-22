//! Panel shell: document, sidebar, brand, theme, and notification view.
//!
//! Renders the Filament-grade shell framing every admin page. Depends only
//! on `argentum-ui` + context — never on [`Resource`].

use super::{Panel, PanelPrefix};

use http::header::COOKIE;
use topcoat::icon::icon;
use topcoat::runtime::{Event, Signal, signal};
use topcoat::view::internal::ThenView;
use topcoat::{
    Result,
    asset::Asset,
    context::{Cx, try_request_context},
    font::Font,
    router::Slot,
    view::{BoxView, Child, HoistView, View, ViewExt, attributes, view},
};

use crate::notification::{LiveToast, live_toast, live_toaster, take_notification};
use crate::resource::NavigationItem;

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

/// Whether the shell starts in dark mode for a visitor with no stored choice.
/// Persisted via `theme.js` (`localStorage` + `theme` cookie).
///
/// Precedence (GH #102, corrected in GH #184): this build-time default only
/// sets the initial `<html class>` and is handed to the blocking
/// `theme_init_script` as its fallback. The stored preference — `localStorage`
/// first, then the `theme` cookie — wins in **both** directions: a stored
/// `light` removes the class this default added. The server never reads the
/// cookie per request, so a toggle is client-side until the next navigation,
/// at which point this default would otherwise re-darken the page — which is
/// exactly the bug GH #184 fixed.
#[derive(Debug, Clone, Copy)]
pub struct DarkMode(pub bool);

/// The application-owned assets used by [`Panel::layout_shell`].
///
/// Tailwind's generated stylesheet is necessarily a call-site asset because
/// every application scans a different source tree. The Panel therefore takes
/// the generated stylesheet and the application's chosen font as values while
/// still owning the document markup that links them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShellAssets {
    pub(crate) stylesheet: Asset,
    pub(crate) font: Font,
}

impl Panel {
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
        // breaking ties — a resource's `navigation()` override interleaves by
        // setting it.
        nav_items.sort_by_key(|item| item.order);
        // A Panel resolves every item it owns (`Panel::resource`); one that
        // reaches the sidebar unresolved has no URL to render, which is a
        // framework bug rather than user error (GH #165).
        debug_assert!(
            nav_items.iter().all(|item| item.url().is_some()),
            "navigation items are resolved by the Panel that owns them"
        );
        let current_path = current_path.to_string();

        Ok(view! {
            cx =>
            sidebar_group(
                sidebar_group_label("Navigation")
                sidebar_group_content(
                    sidebar_menu(
                        for item in &nav_items {
                            let is_active = item.is_current_path(&current_path);
                            sidebar_menu_item(
                                sidebar_menu_button(
                                    active: is_active,
                                    href: item.url(),
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
    ///
    /// Asset note: desktop persistence needs `assets/sidebar.js`
    /// (`argentum_ui::SIDEBAR_JS`), which [`Self::layout_shell`]'s document —
    /// not this function — emits (scripts are owned by the document). The
    /// mobile sheet (`#mobile-sidebar-sheet`) instead dismisses through its
    /// own runtime `@keydown`/`@click` handlers, so it needs no asset; the
    /// vendored `sheet`/`sidebar` primitives carry no note themselves
    /// (ADR-0007 sync guard). See ADR-0014.
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
            .unwrap_or_else(|| "Argentum".to_string());
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
                            (crate::csrf::field(cx, &csrf))
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
        let notification_view: BoxView<'_> = match take_notification(cx) {
            Some(notification) => {
                crate::notification::render_notification(cx, notification, Default::default())
                    .await?
            }
            // No `<span>` placeholder (GH #160): the toaster renders an `<ol>`,
            // which permits only `li`/`script`/`template` children — the empty
            // view renders nothing.
            None => ().boxed(),
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
                    sidebar_header(
                        <div class="flex items-center gap-2">
                            (sidebar_brand)
                            // Below `md` the sheet covers the inset header,
                            // so the sheet's own header carries the close
                            // control (upstream `examples/ui`); the mobile
                            // trigger in the inset header is behind the veil.
                            argentum_ui::button(
                                variant: argentum_ui::ButtonVariant::Ghost,
                                size: argentum_ui::ButtonSize::Icon,
                                attrs: attributes! {
                                    type="button"
                                    class="md:hidden ml-auto"
                                    aria-label="Close sidebar"
                                    @click=$(|_e: Event| mobile_open.set(false))
                                },
                                icon(data: argentum_ui::icons::X)
                            )
                        </div>
                    )
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
                    // `sidebar_inset` is the document's one `<main>`; a second
                    // nested landmark is invalid and confuses landmark
                    // navigation (upstream `examples/ui` uses a plain div).
                    <div class="flex-1 mx-auto max-w-7xl w-full p-6">(slot)</div>
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
        // Prefer declarative nav_items from Panel::resource, fallback to Home.
        let nav_items = try_app_context::<Vec<NavigationItem>>(cx)
            .cloned()
            .unwrap_or_else(|| {
                let prefix = try_app_context::<PanelPrefix>(cx)
                    .map(|p| p.0.clone())
                    .unwrap_or_else(|| {
                        current
                            .split('/')
                            .nth(1)
                            .filter(|s| !s.is_empty())
                            .map(|s| format!("/{s}"))
                            .unwrap_or_else(|| "/admin".to_string())
                    });
                vec![NavigationItem::at("Home", prefix)]
            });
        let shell = Self::render_shell(cx, &nav_items, &current, slot, None).await?;
        let brand_title = try_app_context::<Brand>(cx)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Argentum".to_string());
        Self::render_document(cx, brand_title, shell).await
    }

    /// The complete HTML document around a rendered body: assets, dark-mode
    /// class, and title. [`Self::layout_shell`] frames the panel shell with
    /// it; the standalone login page (ADR-0013) uses the same document so
    /// brand and dark mode carry over.
    ///
    /// The nine shell scripts ship `defer`red (deliberate all-load policy,
    /// ADR-0014): parsing never waits for them, and every one is safe
    /// deferred — document-level listeners install after parse, and the
    /// `DOMContentLoaded` handlers still run, since deferred scripts execute
    /// first. The blocking `theme_init_script` stays inline so the `dark`
    /// class lands pre-paint.
    pub(crate) async fn render_document<'a>(
        cx: &'a Cx,
        title: String,
        body: BoxView<'a>,
    ) -> Result<BoxView<'a>> {
        use topcoat::context::try_app_context;
        // The build-time default: the `<html class>` a first-time visitor gets,
        // and the fallback the blocking script uses when nothing is stored
        // (GH #102, GH #184).
        let default_dark = try_app_context::<DarkMode>(cx).is_some_and(|dm| dm.0);
        let head: BoxView<'_> = match try_app_context::<ShellAssets>(cx).copied() {
            Some(ShellAssets { stylesheet, font }) => view! {
                cx =>
                topcoat::dev::script()
                argentum_ui::theme_init_script(default_dark: default_dark)
                topcoat::runtime::script()
                topcoat::font::link(font: font)
                <link rel="stylesheet" href=(stylesheet)>
                <script src=(argentum_ui::SIDEBAR_JS) defer=""></script>
                <script src=(argentum_ui::THEME_JS) defer=""></script>
                <script src=(argentum_ui::DIALOG_JS) defer=""></script>
                <script src=(argentum_ui::BULK_JS) defer=""></script>
                <script src=(argentum_ui::FILTERS_JS) defer=""></script>
                <script src=(argentum_ui::LIVE_SEARCH_JS) defer=""></script>
                <script src=(argentum_ui::SELECTS_JS) defer=""></script>
                <script src=(argentum_ui::VARIANT_JS) defer=""></script>
                <script src=(argentum_ui::NOTIFICATION_JS) defer=""></script>
            }
            .boxed(),
            None => view! {
                cx =>
                topcoat::dev::script()
                argentum_ui::theme_init_script(default_dark: default_dark)
            }
            .boxed(),
        };
        let html_class = default_dark.then_some("dark");
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn layout_shell_renders_a_complete_document() {
        use crate::resource::{NavTarget, NavigationItem};
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
                target: NavTarget::Url("/admin/users".to_string()),
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
            html.contains("<title>Argentum</title>"),
            "missing document title in {html}"
        );
        assert!(html.contains("hello"), "missing layout slot in {html}");
    }

    #[tokio::test]
    async fn shell_escapes_brand_name_and_logo() {
        use crate::resource::{NavTarget, NavigationItem};
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
            target: NavTarget::Url("/admin/users".to_string()),
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
        // GH #184: the build-time default is only that — the pre-paint script
        // can remove the class again when the visitor has stored `light`.
        assert!(
            html.contains("classList.add") && html.contains("classList.remove"),
            "the document must carry the reconciling theme script, got {html}"
        );

        let html = document_html(&cx_with(Some(false))).await;
        assert!(
            html.contains("<html>"),
            "DarkMode(false) must not set the dark class, got {html}"
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

        // Error + description: the typed toast and the supporting line. The
        // icon's colour is paint (GH #216); `data-type="error"` is what selects
        // the destructive icon, and the `<svg>` proves one rendered.
        let enc = serde_json::to_string(&Notification::error("Boom").description("What happened"))
            .unwrap();
        let html = shell_html_with_flash(&enc).await;
        assert!(
            html.contains("data-type=\"error\"") && html.contains("<svg"),
            "an error toast must carry its type and destructive icon, got {html}"
        );
        assert!(
            html.contains("data-description") && html.contains("What happened"),
            "the description must render, got {html}"
        );
    }

    /// The opening tag that starts at `start`, sliced up to the `>` closing it.
    ///
    /// `Attributes` renders in no guaranteed order (topcoat#122), so a test
    /// locates a tag by whichever attribute it can and asserts on the whole
    /// tag. Quoting is honoured, so a `>` inside an attribute value (Tailwind
    /// selectors and arrow-function handlers both carry them) does not end
    /// the slice.
    fn opening_tag_at(html: &str, start: usize) -> &str {
        let mut quoted = false;
        for (offset, byte) in html.as_bytes()[start..].iter().enumerate() {
            match byte {
                b'"' => quoted = !quoted,
                b'>' if !quoted => return &html[start..start + offset],
                _ => {}
            }
        }
        panic!("unterminated tag at byte {start} in {html}");
    }

    /// Render the shell once with a flash cookie carrying `enc`.
    async fn shell_html_with_flash(enc: &str) -> String {
        use crate::resource::{NavTarget, NavigationItem};
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
            target: NavTarget::Url("/admin/users".to_string()),
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
        // GH #102: `order: -1` interleaves a custom item above the
        // resources; ties keep declaration order.
        use crate::resource::{NavTarget, NavigationItem};
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
                target: NavTarget::Url("/admin/users".to_string()),
                order: 0,
            },
            NavigationItem {
                label: "Showcase".to_string(),
                target: NavTarget::Url("/admin/showcase".to_string()),
                order: -1,
            },
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
            "an order: -1 custom item must precede resources, got {html}"
        );
    }

    #[tokio::test]
    async fn panel_shell_renders_sidebar_with_active_and_tokens() {
        // GH #136: structure/aria only — pixel Token/Tailwind classes live in
        // the showcase (`admin_resource_list_page_serve_seeded_users`), so a
        // restyle does not fail core without a behavior change.
        use crate::resource::{NavTarget, NavigationItem};
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let nav_items = vec![
            NavigationItem {
                label: "Users".to_string(),
                target: NavTarget::Url("/admin/users".to_string()),
                order: 0,
            },
            NavigationItem {
                label: "Showcase".to_string(),
                target: NavTarget::Url("/admin/showcase".to_string()),
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
        // Active highlight + real navigation links (the href prop, not attrs)
        assert!(
            html.contains("data-active=\"true\"") && html.contains("aria-current=\"page\""),
            "missing active highlight in {html}"
        );
        assert!(
            html.contains("<a") && html.contains("href=\"/admin/users\""),
            "navigation must render as links in {html}"
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

    /// `sidebar_inset` renders the document's `<main>`; the slot wrapper must
    /// be a plain element or the document carries two nested landmarks
    /// (invalid HTML, and landmark navigation lists both).
    #[tokio::test]
    async fn shell_has_a_single_main_landmark() {
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &[], "/admin", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert_eq!(
            html.matches("<main").count(),
            1,
            "the shell must expose exactly one main landmark, got {html}"
        );
        let main_tag = opening_tag_at(&html, html.find("<main").expect("main landmark"));
        assert!(
            main_tag.contains("data-sidebar=\"inset\""),
            "the inset stays the main landmark, got {main_tag}"
        );
    }

    /// The sheet header carries a mobile-only close control (upstream
    /// `examples/ui`, `sidebar`'s "Include a close button in the mobile
    /// header"): below `md` the open sheet veils the inset header, so the
    /// mobile trigger there is unreachable and the sheet needs its own.
    #[tokio::test]
    async fn sidebar_sheet_header_carries_a_mobile_close_button() {
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &[], "/admin", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let label = html
            .find("aria-label=\"Close sidebar\"")
            .unwrap_or_else(|| panic!("missing Close sidebar control in {html}"));
        // Find the tag by its label, then assert on the whole opening tag:
        // `Attributes` renders in no guaranteed order (topcoat#122).
        let tag_start = html[..label].rfind('<').expect("close control tag start");
        let tag = opening_tag_at(&html, tag_start);
        // GH #216: `md:hidden` is the responsive class that makes this control
        // mobile-only, and it is paint; what a regression would break is that
        // the close control is a real button wired to the sheet's close hook.
        assert!(
            tag.contains("<button") && tag.contains("data-topcoat-on:click"),
            "close control must be a button wired to close the sheet, got {tag}"
        );
    }

    #[tokio::test]
    async fn toaster_renders_no_stray_span_when_empty() {
        // GH #160: the toaster renders an `<ol>`, which permits only
        // `li`/`script`/`template` children — with no flash notification and
        // no live toast, neither slot may strand a `<span>` in the list.
        use crate::resource::NavigationItem;
        use topcoat::context::CxTestBuilder;
        use topcoat::view::view;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let nav_items: Vec<NavigationItem> = vec![];
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(&cx, &nav_items, "/admin/users", slot, None)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let ol = &html[html.find("<ol").expect("toaster list")..];
        let ol = &ol[..ol.find("</ol>").expect("toaster close") + "</ol>".len()];
        assert!(
            ol.contains("data-sonner-toaster"),
            "sliced the toaster list, got {ol}"
        );
        assert!(
            !ol.contains("<span"),
            "empty toaster must not strand a span in the list, got {ol}"
        );
    }

    #[tokio::test]
    async fn render_shell_extra_class_reaches_the_provider() {
        // GH #137: `extra_class` is the custom-document extension point — a
        // passed class must reach the provider markup.
        use topcoat::context::CxTestBuilder;

        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let nav_items: Vec<NavigationItem> = vec![];
        let slot = view! { cx_ref => "hello" }.boxed().into();
        let html = Panel::render_shell(
            &cx,
            &nav_items,
            "/admin/users",
            slot,
            Some("my-shell".to_string()),
        )
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
        assert!(
            html.contains("my-shell"),
            "extra_class must reach the shell markup, got {html}"
        );
    }

    #[tokio::test]
    async fn collapsed_sidebar_cookie_seeds_the_signal() {
        // The persisted `sidebar_state` cookie seeds the runtime signal's
        // initial value, so the first paint matches the last choice; from
        // hydration on, the browser owns the state (assets/sidebar.js mirrors
        // it back).
        use crate::resource::{NavTarget, NavigationItem};
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
            target: NavTarget::Url("/admin/users".to_string()),
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
}
