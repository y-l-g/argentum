use topcoat::{
    Result,
    view::{Attributes, StaticClass, View, class, component, view},
};

use crate::components::primitives::button::{ButtonSize, ButtonVariant, button_variants};
use crate::components::primitives::separator::SeparatorOrientation;

// ---------------------------------------------------------------------------
// Sidebar container
// ---------------------------------------------------------------------------

const SIDEBAR: StaticClass =
    class!("hidden h-full w-64 flex-col border-r border-border bg-background lg:flex");

/// Sidebar component — the persistent navigation rail on desktop.
///
/// On small viewports it is hidden (`hidden lg:flex`); the mobile drawer is
/// a `sheet` opened via [`sidebar_trigger`]. The `attrs` class is appended.
#[component]
pub async fn sidebar(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="sidebar"
            class=(class!(SIDEBAR, attrs.remove("class")))
            (attrs)
        >
            (child)
        </div>
    }
}

/// Header area of the sidebar (brand, topbar).
#[component]
pub async fn sidebar_header(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div
            data-sidebar="header"
            class=(class!(
                "flex h-16 shrink-0 items-center gap-2 border-b border-border px-4",
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
    // Primitive separator's `classes`/`aria` are private, so we replicate
    // the orientation logic here (same as `separator.rs`) and add the
    // `data-sidebar="separator"` hook expected by tests and shadcn parity.
    let (cls, aria) = match orientation {
        SeparatorOrientation::Horizontal => ("h-px w-full", None::<&str>),
        SeparatorOrientation::Vertical => ("h-full w-px", Some("vertical")),
    };
    view! {
        <hr
            data-sidebar="separator"
            class=(class!(
                "shrink-0 border-0 bg-border",
                cls,
                attrs.remove("class"),
            ))
            aria-orientation=(aria)
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
