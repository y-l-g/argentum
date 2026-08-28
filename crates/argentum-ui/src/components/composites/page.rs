use topcoat::{
    Result,
    view::{Attributes, StaticClass, View, class, component, view},
};

// ---------------------------------------------------------------------------
// Page container — owns max-width, padding and vertical rhythm so pages need
// no Tailwind layout classes. See CONTEXT.md:Page and ADR-0008.
// ---------------------------------------------------------------------------

const PAGE: StaticClass = class!("mx-auto flex w-full max-w-7xl flex-col gap-6 p-6");
const PAGE_HEADER: StaticClass = class!("flex flex-col gap-1.5");
const PAGE_TITLE: StaticClass = class!("text-2xl font-bold tracking-tight text-foreground");
const PAGE_DESCRIPTION: StaticClass = class!("text-sm text-muted-foreground");
const PAGE_CONTENT: StaticClass = class!("flex flex-col gap-6");

/// Standard container for an admin page.
///
/// Owns `max-w-7xl mx-auto p-6 flex flex-col gap-6` so pages declare title
/// and content, not Tailwind layout classes.
///
/// ```ignore
/// page(
///     page_header(page_title("Users") page_description("Manage your users."))
///     page_content(table_view)
/// )
/// ```
#[component]
pub async fn page(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! { <div class=(class!(PAGE, attrs.remove("class"))) (attrs)>(child)</div> }
}

#[component]
pub async fn page_header(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div class=(class!(PAGE_HEADER, attrs.remove("class"))) (attrs)>(child)</div>
    }
}

#[component]
pub async fn page_title(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! { <h1 class=(class!(PAGE_TITLE, attrs.remove("class"))) (attrs)>(child)</h1> }
}

#[component]
pub async fn page_description(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <p class=(class!(PAGE_DESCRIPTION, attrs.remove("class"))) (attrs)>(child)</p>
    }
}

#[component]
pub async fn page_content(#[default] mut attrs: Attributes, #[default] child: View) -> Result {
    view! {
        <div class=(class!(PAGE_CONTENT, attrs.remove("class"))) (attrs)>(child)</div>
    }
}
