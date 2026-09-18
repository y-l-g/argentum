//! A signal-bound text input — an Argentum-owned composite (not part of the
//! synced `topcoat-ui-registry` mirror, so `cargo xtask sync-topcoat-ui` leaves
//! it alone).
//!
//! The upstream [`input`](crate::input) takes [`Attributes`], which stores
//! static attribute values only: a `:value`/`@input` runtime binding cannot
//! ride it. A live form needs both the control's chrome and the two-way
//! signal binding, so this component renders the same control with the
//! binding attached (GH #154 §4).

use topcoat::{
    Result,
    runtime::{Event, Signal},
    view::{Attributes, StaticClass, View, class, component, view},
};

/// The input chrome, mirroring `primitives/input.rs`'s `INPUT` (kept in sync
/// by review; the synced file cannot be imported from).
const INPUT: StaticClass = class!(
    "h-9 w-full min-w-0 rounded-lg border border-border bg-transparent px-3 \
     text-sm transition-colors outline-none \
     placeholder:text-muted-foreground \
     file:mr-3 file:h-full file:border-0 file:bg-transparent file:text-sm file:font-medium \
     focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 \
     aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive \
     focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50",
);

/// A text input whose value is bound to a signal.
///
/// Typing writes the signal (`@input`) and the control shows it (`:value`):
/// the two-way binding a live form reads on the server when a shard re-runs.
/// The `attrs` (such as `type`, `name`, `placeholder`, `id`) are forwarded to
/// the underlying `<input>`; a `class` among them is appended, exactly like
/// [`input`](crate::input).
///
/// ```ignore
/// view! {
///     bound_input(value: name.clone(), attrs: attributes! { type="email" })
/// }
/// ```
#[component]
pub async fn bound_input(
    #[into] value: Signal<String>,
    #[default] mut attrs: Attributes,
) -> Result<impl View> {
    Ok(view! {
        <input
            class=(class!(INPUT, attrs.remove("class")))
            :value=$(value.get())
            @input=$(|e: Event| value.set(e.target.value))
            (attrs)
        >
    })
}
