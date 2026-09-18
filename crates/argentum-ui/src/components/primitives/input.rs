// SYNC: topcoat-ui-registry@0.8.1 sha256:2d46c835a1761ffd503d08add31a30bc167f028c96b9841eb3febdebf6df316b — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).
use topcoat::{
    Result,
    view::{Attributes, StaticClass, View, class, component, view},
};

/// The classes for the [`input`] control.
///
/// The height, text size, radius, and focus ring match the `Md`
/// button, so an input and a button sit flush in a row. File inputs restyle
/// the browser's upload button into quiet, borderless text.
const INPUT: StaticClass = class!(
    "h-9 w-full min-w-0 rounded-lg border border-border bg-transparent px-3 \
     text-sm transition-colors outline-none \
     placeholder:text-muted-foreground \
     file:mr-3 file:h-full file:border-0 file:bg-transparent file:text-sm file:font-medium \
     focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 \
     aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive \
     focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50",
);

/// A text input component.
///
/// The `attrs` (such as `type`, `name`, `placeholder`, `disabled`, or event
/// handlers) are forwarded to the underlying `<input>`; a `class` among them
/// is appended to the computed classes. The input fills its container, so
/// size it through the container or with a width class.
/// Set `aria-invalid="true"` to use the error border and focus ring.
///
/// ```ignore
/// view! {
///     input(attrs: attributes! { type="email" placeholder="you@example.com" })
/// }
/// ```
#[component]
pub async fn input(#[default] mut attrs: Attributes) -> Result<impl View> {
    Ok(view! { <input class=(class!(INPUT, attrs.remove("class"))) (attrs)> })
}
