use http::header::COOKIE;
use topcoat::{
    Result,
    context::{Cx, try_request_context},
    view::{Attributes, StaticClass, View, class, component, view},
};

use crate::components::primitives::button::{ButtonSize, ButtonVariant, button_variants};
use crate::components::primitives::separator::SeparatorOrientation;

// ---------------------------------------------------------------------------
// Provider & Inset — shadcn parity (ADR-0009)
// ---------------------------------------------------------------------------

const PROVIDER: StaticClass =
    class!("group group/sidebar-wrapper flex min-h-svh w-full");

/// The sidebar's persisted state from the `sidebar_state` cookie, defaulting
/// to expanded. Returns the `data-state` value plus the matching
/// `data-collapsible` value — "offcanvas" only while collapsed, so
/// `group-data-[collapsible=offcanvas]` rules do not fire when expanded.
///
/// Parsed from the raw `Cookie` header on purpose: `topcoat::cookie::cookies`
/// panics when the cookie router layer is absent (tests, minimal routers),
/// and the shell must render everywhere.
fn sidebar_state(cx: &Cx) -> (&'static str, &'static str) {
    let collapsed = try_request_context::<http::request::Parts>(cx)
        .and_then(|parts| parts.headers.get(COOKIE))
        .and_then(|v| v.to_str().ok())
        .is_some_and(|cookie| {
            cookie
                .split(';')
                .any(|part| part.trim().strip_prefix("sidebar_state=") == Some("collapsed"))
        });
    if collapsed {
        ("collapsed", "offcanvas")
    } else {
        ("expanded", "")
    }
}

/// Sidebar provider — sets CSS vars for width and wraps the entire shell.
///
/// Mirrors `SidebarProvider` in `ui/apps/v4/registry/new-york-v4/ui/sidebar.tsx`:
/// `--sidebar-width:16rem`, `--sidebar-width-icon:3rem`,
/// `--sidebar-width-mobile:18rem`, `group/sidebar-wrapper flex min-h-svh w-full`,
/// `data-state` + `data-collapsible` for `group-data-[collapsible=icon]` rules.
/// Reads `sidebar_state` cookie via `http::request::Parts` in `Cx` for SSR.
#[component]
pub async fn sidebar_provider(
    cx: &Cx,
    #[default] mut attrs: Attributes,
    #[default] child: View,
) -> Result {
    let (state, collapsible) = sidebar_state(cx);
    view! {
        <div
            data-sidebar="provider"
            data-state=(state)
            data-collapsible=(collapsible)
            style="--sidebar-width:16rem;--sidebar-width-icon:3rem;--sidebar-width-mobile:18rem"
            class=(class!(PROVIDER, attrs.remove("class")))
            (attrs)
        >
            (child)
        </div>
    }
}

const INSET: StaticClass = class!("flex flex-1 flex-col min-w-0");

/// Inset for the main content beside the fixed sidebar (shadcn `SidebarInset`).
#[component]
pub async fn sidebar_inset(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div data-sidebar="inset" class=(class!(INSET, attrs.remove("class"))) (attrs)>
            (child)
        </div>
    }
}

// ---------------------------------------------------------------------------
// Sidebar container
// ---------------------------------------------------------------------------

const SIDEBAR: StaticClass = class!(
    "fixed inset-y-0 left-0 z-10 hidden h-svh w-(--sidebar-width) flex-col border-r border-border bg-background transition-[left,right,width,background-color,border-color,color] duration-200 group-data-[collapsible=offcanvas]:left-[calc(var(--sidebar-width)*-1)] group-data-[collapsible=icon]:w-(--sidebar-width-icon) lg:flex"
);

/// Sidebar component — the persistent navigation rail on desktop.
///
/// On small viewports it is hidden (`hidden lg:flex`); the mobile drawer is
/// a `sheet` opened via [`sidebar_trigger`]. `fixed inset-y-0 h-svh w-(--sidebar-width)`
/// with `data-state` + `data-collapsible` derived from the `sidebar_state`
/// cookie — the same state [`sidebar_provider`] ships, so SSR markup is
/// consistent across the shell.
#[component]
pub async fn sidebar(cx: &Cx, #[default] mut attrs: Attributes, #[default] child: View) -> Result {
    let (state, collapsible) = sidebar_state(cx);
    view! {
        <div
            data-sidebar="sidebar"
            data-state=(state)
            data-collapsible=(collapsible)
            data-variant="sidebar"
            class=(class!(SIDEBAR, attrs.remove("class")))
            (attrs)
        >
            (child)
        </div>
    }
}

/// Header area of the sidebar (brand, topbar) — sticky per shadcn.
#[component]
pub async fn sidebar_header(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="header"
            class=(class!(
                "sticky top-0 z-10 flex h-16 shrink-0 items-center gap-2 border-b border-border bg-background px-4",
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </div>
    }
}

/// Main scrollable area of the sidebar.
#[component]
pub async fn sidebar_content(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="content"
            class=(class!(
                "flex flex-1 flex-col gap-2 overflow-y-auto p-2",
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </div>
    }
}

/// Footer area of the sidebar.
#[component]
pub async fn sidebar_footer(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="footer"
            class=(class!(
                "flex shrink-0 flex-col gap-2 border-t border-border p-2",
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </div>
    }
}

// ---------------------------------------------------------------------------
// Group
// ---------------------------------------------------------------------------

#[component]
pub async fn sidebar_group(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="group"
            class=(class!("flex flex-col gap-2", attrs.remove("class")))
            (attrs)
        >
            (child)
        </div>
    }
}

#[component]
pub async fn sidebar_group_label(
    #[default] mut attrs: Attributes,
    #[default] child: View,
) -> Result {
    view! {
        <div
            data-sidebar="group-label"
            class=(class!(
                "px-2 py-1 text-xs font-medium text-muted-foreground group-data-[collapsible=icon]:hidden",
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </div>
    }
}

#[component]
pub async fn sidebar_group_content(
    #[default] mut attrs: Attributes,
    #[default] child: View,
) -> Result {
    view! {
        <div
            data-sidebar="group-content"
            class=(class!("flex flex-col gap-1", attrs.remove("class")))
            (attrs)
        >
            (child)
        </div>
    }
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

#[component]
pub async fn sidebar_menu(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <ul
            data-sidebar="menu"
            class=(class!("flex flex-col gap-1", attrs.remove("class")))
            (attrs)
        >
            (child)
        </ul>
    }
}

#[component]
pub async fn sidebar_menu_item(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <li
            data-sidebar="menu-item"
            class=(class!("list-none", attrs.remove("class")))
            (attrs)
        >
            (child)
        </li>
    }
}

const MENU_BUTTON_BASE: StaticClass = class!(
    "flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm font-medium \
     transition-colors hover:bg-foreground/5 hover:text-foreground \
     focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 \
     focus-visible:ring-offset-background outline-none \
     group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-2"
);

const MENU_BUTTON_ACTIVE: StaticClass = class!("bg-foreground/5 text-foreground");
const MENU_BUTTON_INACTIVE: StaticClass = class!("text-muted-foreground");

/// A single navigation button inside the sidebar.
///
/// `is_active` controls the active styling (`bg-foreground/5` + `aria-current="page"`)
/// and hover is `Ghost`.
#[component]
pub async fn sidebar_menu_button(
    #[default] is_active: bool,
    #[default] mut attrs: Attributes,
    #[default] child: View,
) -> Result {
    let active_class = if is_active {
        MENU_BUTTON_ACTIVE
    } else {
        MENU_BUTTON_INACTIVE
    };
    view! {
        <a
            data-sidebar="menu-button"
            data-active=(is_active.then_some("true"))
            aria-current=(is_active.then_some("page"))
            class=(class!(MENU_BUTTON_BASE, active_class, attrs.remove("class")))
            (attrs)
        >
            (child)
        </a>
    }
}

// ---------------------------------------------------------------------------
// Separator & Trigger
// ---------------------------------------------------------------------------

/// Sidebar separator — wraps `separator` with sidebar semantics.
#[component]
pub async fn sidebar_separator(
    #[default] orientation: SeparatorOrientation,
    #[default] mut attrs: Attributes,
) -> Result {
    view! {
        <hr
            data-sidebar="separator"
            class=(class!(
                "shrink-0 border-0 bg-border",
                orientation.classes(),
                attrs.remove("class"),
            ))
            aria-orientation=(orientation.aria())
            (attrs)
        >
    }
}

/// Trigger that toggles the sidebar on mobile (opens `sheet` drawer) or
/// collapses the desktop rail. Renders a Ghost icon button with
/// `data-collapsed` hook and `lg:hidden` / `hidden lg:flex` responsive
/// classes as appropriate. Persistence via cookie/session is handled by the
/// consumer; this component only renders the hook.
#[component]
pub async fn sidebar_trigger(#[default] mut attrs: Attributes) -> Result {
    view! {
        <button
            data-sidebar="trigger"
            aria-label="Toggle sidebar"
            class=(class!(
                button_variants(ButtonVariant::Ghost, ButtonSize::Icon),
                attrs.remove("class"),
            ))
            (attrs)
        >
            <span aria-hidden="true">"☰"</span>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;

    fn cx_with_cookie(value: &str) -> Cx {
        let (parts, ()) = http::Request::builder()
            .uri("/")
            .header(http::header::COOKIE, value)
            .body(())
            .unwrap()
            .into_parts();
        CxTestBuilder::new().request_context(parts).build()
    }

    #[tokio::test]
    async fn provider_and_sidebar_agree_on_collapsed_cookie() {
        let cx = cx_with_cookie("theme=dark; sidebar_state=collapsed; other=1");
        let cx_ref = &cx;
        let provider = view! { cx_ref => sidebar_provider() }.unwrap().render(&cx);
        let rail = view! { cx_ref => sidebar() }.unwrap().render(&cx);
        for html in [provider, rail] {
            assert!(
                html.contains("data-state=\"collapsed\"")
                    && html.contains("data-collapsible=\"offcanvas\""),
                "collapsed state missing from {html}"
            );
        }
    }

    #[tokio::test]
    async fn malformed_cookie_falls_back_to_expanded() {
        let cx = cx_with_cookie("sidebar_state=pwned");
        let cx_ref = &cx;
        let provider = view! { cx_ref => sidebar_provider() }.unwrap().render(&cx);
        let rail = view! { cx_ref => sidebar() }.unwrap().render(&cx);
        for html in [provider, rail] {
            assert!(
                html.contains("data-state=\"expanded\"")
                    && html.contains("data-collapsible=\"\""),
                "expected expanded fallback in {html}"
            );
        }
    }

    #[tokio::test]
    async fn no_request_context_renders_expanded() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let rail = view! { cx_ref => sidebar() }.unwrap().render(&cx);
        assert!(rail.contains("data-state=\"expanded\""), "{rail}");
    }
}
