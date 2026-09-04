// SYNC: topcoat-ui-registry@0.6.2 sha256:c5cb2baa25c978591e222d9e253b6d2c347f48c339bc5f348e1c54e70aaca620 — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).
use topcoat::{
    Result,
    icon::{icon, iconify::iconify_icon},
    view::{Attributes, StaticClass, View, attributes, class, component, view},
};

/// The classes for the native `<input type="checkbox">` inside the
/// [`checkbox`] component.
///
/// The native glyph is suppressed with `appearance-none` so the component can
/// draw its own checkmark, which keeps the control looking the same across
/// browsers. The unchecked box matches the input control's border and shadow;
/// checking it fills the box with the primary color.
const CHECKBOX: StaticClass = class!(
    "peer size-4 shrink-0 appearance-none rounded-[4px] border border-border \
     bg-background shadow-xs transition-colors outline-none \
     checked:border-primary checked:bg-primary \
     focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 \
     focus-visible:ring-offset-background disabled:pointer-events-none",
);

/// A checkbox component: a themed native `<input type="checkbox">`.
///
/// The `attrs` (such as `name`, `checked`, `disabled`, or event handlers) are
/// forwarded to the `<input>`; a `class` among them is appended to the
/// wrapping element's classes. Set the checked state with a plain `checked`
/// attribute. The indeterminate state is not styled: it is only reachable
/// through the DOM property, so setting it takes a script to begin with.
///
/// ```ignore
/// view! {
///     <div class="flex items-center gap-2">
///         checkbox(attrs: attributes! { id="terms" name="terms" checked="" })
///         label(attrs: attributes! { for="terms" }, "Accept terms")
///     </div>
/// }
/// ```
#[component]
pub async fn checkbox(#[default] mut attrs: Attributes) -> Result<impl View> {
    // The checkmark cannot be drawn by the `<input>` itself, which renders no
    // children or pseudo-elements: it is a sibling icon overlaid on the
    // control, revealed by the input's `peer` state while checked.
    Ok(view! {
        <span
            class=(class!(
                "relative inline-flex shrink-0 has-[:disabled]:opacity-50",
                attrs.remove("class"),
            ))
        >
            <input type="checkbox" class=(CHECKBOX) (attrs)>
            icon(
                data: iconify_icon!("lucide:check"),
                attrs: attributes! {
                    class="pointer-events-none absolute inset-0 m-auto size-3.5 \
                        text-primary-foreground opacity-0 peer-checked:opacity-100"
                }
            )
        </span>
    })
}
