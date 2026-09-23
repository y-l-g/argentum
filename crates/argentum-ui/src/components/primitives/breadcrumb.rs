// SYNC: topcoat-ui-registry@0.8.1 sha256:c687399a226cf6cd05b015f95d5bbe00a314d7176c540f4362dadd9c0424e22c — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).
use topcoat::{
    Result,
    icon::{icon, iconify::iconify_icon},
    view::{Attributes, Child, View, attributes, class, component, view},
};

/// Navigation links showing the path to the current page.
///
/// Place links in a `breadcrumb_list` and use `breadcrumb_page` for the current page.
/// `attrs` are forwarded to the `<nav>`, with extra classes added to its classes.
///
/// ```ignore
/// view! {
///     breadcrumb(
///         breadcrumb_list(
///             breadcrumb_item(
///                 breadcrumb_link(attrs: attributes! { href="/" }, "Home")
///             )
///             breadcrumb_separator()
///             breadcrumb_item(breadcrumb_page("Settings"))
///         )
///     )
/// }
/// ```
#[component]
pub async fn breadcrumb(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <nav aria-label="breadcrumb" class=(attrs.remove("class")) (attrs)>(child)</nav>
    })
}

/// The ordered list of steps in a [`breadcrumb`].
///
/// The steps wrap onto another line rather than overflowing when the trail
/// outgrows its container.
#[component]
pub async fn breadcrumb_list(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <ol
            class=(class!(
                "flex flex-wrap items-center gap-2 text-sm text-muted-foreground",
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </ol>
    })
}

/// One step of a [`breadcrumb_list`], holding a [`breadcrumb_link`] or a
/// [`breadcrumb_page`].
#[component]
pub async fn breadcrumb_item(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <li
            class=(class!("inline-flex items-center gap-2", attrs.remove("class")))
            (attrs)
        >
            (child)
        </li>
    })
}

/// A link to an ancestor page. Pass its destination as `href` in `attrs`.
#[component]
pub async fn breadcrumb_link(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <a
            class=(class!(
                "transition-colors hover:text-foreground",
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </a>
    })
}

/// The current page label.
///
/// Renders with `aria-current="page"` and does not navigate.
#[component]
pub async fn breadcrumb_page(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <span
            aria-current="page"
            class=(class!("font-medium text-foreground", attrs.remove("class")))
            (attrs)
        >
            (child)
        </span>
    })
}

/// A decorative chevron between breadcrumb items, hidden from assistive technology.
#[component]
pub async fn breadcrumb_separator(#[default] mut attrs: Attributes) -> Result<impl View> {
    Ok(view! {
        <li aria-hidden="true" class=(attrs.remove("class")) (attrs)>
            icon(
                data: iconify_icon!("lucide:chevron-right"),
                attrs: attributes! { class="size-3.5" }
            )
        </li>
    })
}

/// An ellipsis representing omitted breadcrumb items, with an accessible text label.
#[component]
pub async fn breadcrumb_ellipsis(#[default] mut attrs: Attributes) -> Result<impl View> {
    Ok(view! {
        <span class=(class!("flex items-center", attrs.remove("class"))) (attrs)>
            icon(
                data: iconify_icon!("lucide:ellipsis"),
                attrs: attributes! { class="size-4" }
            )
            <span class="sr-only">"More"</span>
        </span>
    })
}
