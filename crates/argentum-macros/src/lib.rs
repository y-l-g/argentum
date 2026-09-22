//! Procedural macros for Argentum.

mod embedded;

use proc_macro::TokenStream;
use syn::DeriveInput;

/// Derive `EmbeddedForm` for an embedded struct or enum (GH #191).
///
/// The generated impl converts the value to and from the panel's flat form map,
/// answers whether a submission mentions it, and generates a `form(cx, parent)`
/// function returning the value's controls — all driven by the columns the app
/// schema resolves for the parent path. The framework supplies the storage
/// names, this derive supplies the Rust shape, so no flattened name is ever
/// spelled by hand.
///
/// ```ignore
/// #[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
/// pub enum Publication {
///     #[column(variant = 1)]
///     Scheduled {
///         #[shared(timestamp)]
///         #[form(label = "Publication timestamp")]
///         scheduled_at: String,
///         scheduled_for: String,
///     },
///     #[column(variant = 2)]
///     Published {
///         #[shared(timestamp)]
///         published_at: String,
///         canonical_url: String,
///     },
/// }
///
/// // form declaration — no field bindings written by hand
/// Section::new("Publication").schema(Publication::form(cx, Post::fields().publication()))
///
/// // hydration and the record fn
/// write_embedded(cx, Post::fields().publication(), &record.publication, &mut values);
/// let publication = read_embedded(cx, Post::fields().publication(), &values);
/// ```
///
/// # How a field is classified
///
/// A field whose type is a Rust primitive this panel can spell — `String`, the
/// integer family, `bool`, `f32`/`f64`, `Uuid`, `jiff::Timestamp` — is a
/// **leaf**: one column, read and written as text (typed leaves parse through
/// `TypedValue`, GH #192). Any other type is another **embedded value**,
/// delegated to that type's own `EmbeddedForm` impl, so nesting works by
/// deriving on each type. A relation, an `Option<T>`, a `Vec<T>`, and a
/// `#[document]` inside a value do not compile, or are refused at the schema
/// (see the `argentum-core` module docs).
///
/// # Which variant an enum reads
///
/// The discriminant column decides, in this order:
///
/// 1. a discriminant the submission **names** — always wins, and one the enum
///    does not declare is refused loudly rather than read as some other
///    variant;
/// 2. otherwise (the create form, a hand-written POST) the first variant, in
///    declaration order, with a **payload of its own** submitted — a
///    `#[shared(..)]` column belongs to several variants and so never selects
///    one;
/// 3. otherwise the first variant.
///
/// # Per-field overrides
///
/// - `#[form(label = "Canonical URL")]` — the control's label (default: the
///   field name, humanized).
/// - `#[form(textarea)]` / `#[form(textarea, rows = 3)]` — a multi-line control
///   for a `String` leaf, and its height.
///
/// Anything else in `#[form(..)]` is a compile error.
///
/// # Not covered
///
/// A `#[document]` **inside** an embedded value (its inner fields share one
/// column, so no per-field binding exists), a relation inside one, an embedded
/// enum nested **inside an enum variant** (value resolution starts at a model
/// root), and a tuple or unit struct.
#[proc_macro_derive(EmbeddedForm, attributes(form))]
pub fn embedded_form(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    embedded::expand(input)
}
