// SYNC: topcoat-ui-registry@0.9.0 sha256:6741fe1cf51bea5cfe0d0c8d25548e452661d9017529a7163bc5b60ce725a8d5 — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).
use topcoat::{
    Result,
    view::{Attributes, Child, StaticClass, View, class, component, view},
};

/// Classes that align label content and reflect the disabled state of a nearby or
/// nested control.
const LABEL: StaticClass = class!(
    "flex items-center gap-2 text-sm leading-none font-medium select-none \
     peer-disabled:pointer-events-none peer-disabled:opacity-50 \
     peer-has-[:disabled]:pointer-events-none peer-has-[:disabled]:opacity-50 \
     has-[+:disabled]:pointer-events-none has-[+:disabled]:opacity-50 \
     has-[:disabled]:pointer-events-none has-[:disabled]:opacity-50",
);

/// A label for a form control.
///
/// Wrap the control or pass a `for` attribute matching its `id`. Pass the label text as
/// children. `attrs` are forwarded to the `<label>`, with extra classes added to its
/// classes.
///
/// ```ignore
/// view! {
///     <div class="flex flex-col gap-2">
///         label(attrs: attributes! { for="email" }, "Email")
///         input(attrs: attributes! { id="email" type="email" })
///     </div>
/// }
/// ```
#[component]
pub async fn label(
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <label class=(class!(LABEL, attrs.remove("class"))) (attrs)>(child)</label>
    })
}
