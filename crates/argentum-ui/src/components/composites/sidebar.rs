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

const PROVIDER: StaticClass = class!("group/sidebar-wrapper flex min-h-svh w-full");

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
    let state = try_request_context::<http::request::Parts>(cx)
        .and_then(|parts| parts.headers.get(COOKIE))
        .and_then(|v| v.to_str().ok())
        .and_then(|cookie| {
            for part in cookie.split(';') {
                let trimmed = part.trim();
                if let Some(val) = trimmed.strip_prefix("sidebar_state=") {
                    if val == "collapsed" || val == "expanded" {
                        return Some(val);
                    }
                }
            }
            None
        })
        .unwrap_or("expanded");
    view! {
        <div
            data-sidebar="provider"
            data-state=(state)
            data-collapsible=""
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
        <div
            data-sidebar="inset"
            class=(class!(INSET, attrs.remove("class")))
            (attrs)
        >
            (child)
        </div>
    }
}

// ---------------------------------------------------------------------------
// Sidebar container
// ---------------------------------------------------------------------------

const SIDEBAR: StaticClass = class!(
    "fixed inset-y-0 z-10 hidden h-svh w-(--sidebar-width) flex-col border-r border-border bg-background transition-[left,right,width] lg:flex"
);

/// Sidebar component — the persistent navigation rail on desktop.
///
/// On small viewports it is hidden (`hidden lg:flex`); the mobile drawer is
/// a `sheet` opened via [`sidebar_trigger`]. `fixed inset-y-0 h-svh w-(--sidebar-width)`
/// with `data-state` + `data-collapsible` for `group-data-[collapsible=icon]` rules.
#[component]
pub async fn sidebar(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="sidebar"
            data-state="expanded"
            data-collapsible=""
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
                "px-2 py-1 text-xs font-medium text-muted-foreground",
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
     focus-visible:ring-offset-background outline-none"
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
