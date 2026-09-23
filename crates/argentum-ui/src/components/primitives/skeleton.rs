// SYNC: topcoat-ui-registry@0.8.1 sha256:3b6d99ca9aef02697ce1142903bd49062ef9f2bd4ecbb1e6b82be0ea6554d1a5 — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).
use topcoat::{
    Result,
    view::{Attributes, StaticClass, View, class, component, view},
};

/// Classes for a pulsing placeholder with a muted background.
const SKELETON: StaticClass = class!("animate-pulse rounded-md bg-foreground/10");

/// A pulsing placeholder for content that is loading.
///
/// Set its dimensions through classes in `attrs`. Match the expected content's shape to
/// reduce layout movement when it arrives. Attributes are forwarded to the `<div>`,
/// with extra classes added to its classes.
///
/// ```ignore
/// view! {
///     <div class="flex flex-col gap-2">
///         skeleton(attrs: attributes! { class="h-4 w-32" })
///         skeleton(attrs: attributes! { class="h-4 w-full" })
///     </div>
/// }
/// ```
#[component]
pub async fn skeleton(#[default] mut attrs: Attributes) -> Result<impl View> {
    Ok(view! { <div class=(class!(SKELETON, attrs.remove("class"))) (attrs)></div> })
}
