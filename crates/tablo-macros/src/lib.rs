//! Procedural macros for Tablo.

mod embedded;

use proc_macro::TokenStream;
use syn::DeriveInput;

/// Derive `EmbeddedForm` for an embedded struct or enum.
///
/// The generated impl converts the value to and from the panel's flat form map,
/// answers whether a submission mentions it, and generates a `form(cx, parent)`
/// returning the value's controls, all driven by the columns the app schema
/// resolves for the parent path. The framework supplies the storage names, the
/// derive supplies the Rust shape.
///
/// ```ignore
/// #[derive(Debug, Clone, toasty::Embed, tablo_core::EmbeddedForm)]
/// pub enum Publication {
///     #[column(variant = 1)]
///     Scheduled {
///         #[shared(timestamp)]
///         #[form(label = "Publication timestamp")]
///         scheduled_at: String,
///         scheduled_for: String,
///     },
///     #[column(variant = 2)]
///     Published { #[shared(timestamp)] published_at: String, canonical_url: String },
/// }
///
/// // form declaration — no field bindings written by hand
/// Section::new("Publication").schema(Publication::form(cx, Post::fields().publication()))
/// ```
///
/// # How a field is classified
///
/// A type this panel can spell — `String`, the integer family, `bool`,
/// `f32`/`f64`, `Uuid`, `jiff::Timestamp` — is a **leaf**: one column, read and
/// written as text (typed leaves parse through `TypedValue`). Any other type is
/// another **embedded value**, delegated to that type's own `EmbeddedForm`. A
/// relation, an `Option<T>`, a `Vec<T>` and a `#[document]` inside a value do
/// not compile, or are refused at the schema (the `tablo-core`
/// `schema::embedded` module docs list what is not covered).
///
/// # Which variant an enum reads
///
/// A named discriminant always wins, and an undeclared one is refused loudly;
/// otherwise the first variant, in declaration order, with a **payload of its
/// own** submitted — a `#[shared(..)]` column belongs to several variants and
/// never selects one; otherwise the first variant.
///
/// # Per-field overrides
///
/// - `#[form(label = "Canonical URL")]` — the control's label (default: the field name, humanized).
/// - `#[form(textarea)]` / `#[form(textarea, rows = 3)]` — a multi-line control for a `String`
///   leaf, and its height.
///
/// Anything else in `#[form(..)]` is a compile error.
#[proc_macro_derive(EmbeddedForm, attributes(form))]
pub fn embedded_form(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    embedded::expand(input)
}
