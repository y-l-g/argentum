//! Procedural macros for Argentum.

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
/// #[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
/// pub struct Seo { pub title: String, pub description: String }
///
/// Section::new("SEO").schema(Seo::form(cx, Post::fields().seo()));
/// write_embedded(cx, Post::fields().seo(), &record.seo, &mut values);
/// let seo = read_embedded(cx, Post::fields().seo(), &values);
/// ```
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
/// Anything else in `#[form(..)]` is a compile error. See the `argentum-core`
/// module docs for how a field is classified and what is not covered.
#[proc_macro_derive(EmbeddedForm, attributes(form))]
pub fn embedded_form(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    embedded::expand(input)
}
