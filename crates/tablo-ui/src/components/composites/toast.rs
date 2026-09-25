//! Toast — the shadcn/Sonner surface, owned by Tablo.
//!
//! The parts mirror shadcn's Sonner composition: a [`toaster`] stack, the
//! [`toast`] surface, [`toast_icon`], [`toast_content`] with [`toast_title`] /
//! [`toast_description`], and [`toast_close`]. The classes follow shadcn's
//! new-york Sonner theming (`bg-background text-foreground border-border
//! shadow-lg`, description `text-muted-foreground`) over Sonner's own layout
//! values (356px stack, 16px padding, 13px text, 14px gap, 20px circular
//! close button, bottom-right offset).
//!
//! The enter/exit motion is Sonner's: the server renders `data-mounted="false"`
//! (slid down, transparent), `notifications.js` flips it to `data-mounted="true"`
//! to play the transition, and flips `data-removed="true"` before removing the
//! toast. A `<noscript>` rule keeps toasts visible without JS, where
//! auto-dismissal and the close button only work by navigation.

use topcoat::{
    Result,
    icon::icon,
    view::{Attributes, Child, StaticClass, View, attributes, class, component, view},
};

use crate::icons;

/// The fixed stack Sonner calls the toaster: bottom-right, 356px, 14px gap.
///
/// The viewport offset is Sonner's: 16px on small screens, 24px from `sm` up.
/// The stack takes no pointer events so the empty area never blocks the page;
/// each toast re-enables them.
const TOASTER: StaticClass = class!(
    "pointer-events-none fixed right-4 bottom-4 z-50 flex w-[356px] \
     max-w-[calc(100vw-2rem)] flex-col gap-3.5 sm:right-6 sm:bottom-6",
);

/// Sonner's styled toast surface under shadcn's new-york theming.
///
/// `data-mounted` / `data-removed` drive the Sonner transition; the base state
/// is the settled one so a toast without JS is simply visible.
const TOAST: StaticClass = class!(
    "pointer-events-auto relative flex w-full translate-y-0 items-center gap-1.5 rounded-lg \
     border border-border bg-background p-4 text-[13px] text-foreground shadow-lg \
     [overflow-wrap:anywhere] \
     transition-[translate,opacity] duration-[400ms] ease-[cubic-bezier(0.25,0.1,0.25,1)] \
     focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none \
     data-[mounted=false]:translate-y-full data-[mounted=false]:opacity-0 \
     data-[removed=true]:translate-y-full data-[removed=true]:opacity-0",
);

/// Sonner's 16px leading icon slot.
const ICON: StaticClass = class!("flex size-4 shrink-0 items-center justify-center");

/// Sonner's content column: title over description, taking the free width.
const CONTENT: StaticClass = class!("flex min-w-0 flex-1 flex-col gap-0.5");

/// Sonner's title: medium weight, 1.5 line-height.
const TITLE: StaticClass = class!("font-medium leading-normal");

/// Sonner's description under shadcn's theming.
const DESCRIPTION: StaticClass = class!("leading-snug text-muted-foreground");

/// Sonner's close button: a 20px circle hanging off the top-left corner.
const CLOSE: StaticClass = class!(
    "absolute top-0 left-0 flex size-5 -translate-x-[35%] -translate-y-[35%] \
     cursor-pointer items-center justify-center rounded-full border border-border \
     bg-background text-foreground transition-colors hover:bg-muted",
);

/// The toast stack (shadcn/Sonner's toaster).
///
/// Renders Sonner's polite live region around the `<ol data-sonner-toaster>`;
/// the `<noscript>` rule re-reveals toasts whose enter transition never got its
/// `data-mounted` flip.
///
/// Needs `assets/notifications.js` (`crate::NOTIFICATION_JS`, hooks
/// `data-sonner-toast` / `data-close-button`), emitted by
/// `Panel::render_document` on every document with shell assets (ADR-0014).
/// Without the script the toast stays hidden at `data-mounted="false"`; with
/// scripting disabled the `<noscript>` rule keeps it visible.
///
/// ```ignore
/// toaster(
///     toast(attrs: attributes! { data-type="success" },
///         toast_icon(icon(data: icons::CIRCLE_CHECK))
///         toast_content(
///             toast_title("Created")
///             toast_description("The record is live.")
///         )
///         toast_close()
///     )
/// )
/// ```
#[component]
pub async fn toaster(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <section
            aria-label="Notifications"
            tabindex="-1"
            aria-live="polite"
            aria-relevant="additions text"
            aria-atomic="false"
        >
            <noscript>
                <style>
                    "[data-sonner-toast]{opacity:1 !important;transform:none !important}"
                </style>
            </noscript>
            <ol
                class=(class!(TOASTER, attrs.remove("class")))
                data-sonner-toaster=""
                data-y-position="bottom"
                data-x-position="right"
                (attrs)
            >
                (child)
            </ol>
        </section>
    })
}

/// A toast surface, Sonner's `[data-sonner-toast]`.
///
/// Start from `data-mounted="false"` so the enter transition plays when
/// `notifications.js` mounts it; a toast without JS stays visible through the
/// toaster's `<noscript>` rule.
#[component]
pub async fn toast(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <li
            tabindex="0"
            class=(class!(TOAST, attrs.remove("class")))
            data-sonner-toast=""
            data-styled="true"
            data-mounted="false"
            data-visible="true"
            data-y-position="bottom"
            data-x-position="right"
            data-front="true"
            (attrs)
        >
            (child)
        </li>
    })
}

/// The leading status icon slot, Sonner's `[data-icon]`.
#[component]
pub async fn toast_icon(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <div
            class=(class!(ICON, attrs.remove("class")))
            data-icon=""
            aria-hidden="true"
            (attrs)
        >
            (child)
        </div>
    })
}

/// The title/description column, Sonner's `[data-content]`.
#[component]
pub async fn toast_content(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <div class=(class!(CONTENT, attrs.remove("class"))) data-content="" (attrs)>
            (child)
        </div>
    })
}

/// The toast title, Sonner's `[data-title]`.
#[component]
pub async fn toast_title(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <div class=(class!(TITLE, attrs.remove("class"))) data-title="" (attrs)>
            (child)
        </div>
    })
}

/// The supporting line under the title, Sonner's `[data-description]`.
#[component]
pub async fn toast_description(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <div
            class=(class!(DESCRIPTION, attrs.remove("class")))
            data-description=""
            (attrs)
        >
            (child)
        </div>
    })
}

/// Sonner's circular close button, `[data-close-button]`.
#[component]
pub async fn toast_close(#[default] mut attrs: Attributes) -> Result<impl View> {
    Ok(view! {
        <button
            type="button"
            aria-label="Close toast"
            class=(class!(CLOSE, attrs.remove("class")))
            data-close-button=""
            (attrs)
        >
            icon(data: icons::X, attrs: attributes! { class="size-3" })
        </button>
    })
}
