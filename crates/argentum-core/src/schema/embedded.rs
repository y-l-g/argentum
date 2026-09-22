//! First-class embedded **values** (GH #191): a typed value and the flat form
//! map, converted in one declared place.
//!
//! Embedded *leaves* bind one column at a time (GH #185): a lens through an
//! embedded struct, an enum variant, or a `#[document]` resolves to its
//! flattened storage column and a `TextInput` posts it. The **value** those
//! leaves belong to had no seam at all — every app reassembled it by hand from
//! the flat map, and the enum's variant was recovered from *which payload
//! columns happened to be non-empty*:
//!
//! ```ignore
//! let publication = if !field(values, "publication_canonical_url").is_empty() {
//!     Publication::Published { published_at: timestamp, canonical_url: … }
//! } else if … // and so on, per app, per type
//! ```
//!
//! That inference is the bug this module removes. An embedded enum *has* a
//! discriminant column — Toasty stores the variant there — so the form carries
//! the discriminant explicitly, hydration writes the stored variant, and a
//! submission names the variant it means. Nothing inspects payload emptiness.
//!
//! # What an app writes
//!
//! Derive [`EmbeddedForm`] on the embedded type and call the four entry points:
//!
//! ```ignore
//! #[derive(Clone, toasty::Embed, argentum_core::EmbeddedForm)]
//! pub struct Seo { pub title: String, pub description: String }
//!
//! // form declaration: the controls, derived from the metadata
//! Section::new("SEO").schema(Seo::form(cx, Post::fields().seo()))
//!
//! // hydration (edit/view): the record's value, written under its columns
//! write_embedded(cx, Post::fields().seo(), &record.seo, &mut values);
//!
//! // record fn: the value back, with its variant chosen by discriminant
//! let seo = read_embedded(cx, Post::fields().seo(), &values);
//! ```
//!
//! Every key comes from the resolver — the compiled mapping names the column,
//! exactly as it does for a leaf (GH #185). The app never spells a flattened
//! name, and nothing here re-derives Toasty's naming.
//!
//! # What is not covered
//!
//! - A `#[document]` **inside** an embedded value: its fields share one column,
//!   so there is no typed projection to bind (the document type would need its
//!   own codec). Leaf binding of a document still works (GH #185).
//! - A relation inside an embedded value: relations are not stored in the row.
//! - The variant **control**: this module carries the discriminant and derives
//!   the leaf controls; choosing a variant in the UI is the follow-up on GH #191
//!   (the active variant's fields rendering alone needs a form-reactivity seam).

use std::collections::HashMap;

use toasty::stmt::Path;
use topcoat::context::Cx;

use crate::schema::TextInput;
use crate::schema::lenses::FieldResolver;

/// An embedded value that can be read from, and written to, the flat form map
/// (GH #191).
///
/// Derive it with [`argentum_core::EmbeddedForm`](crate::EmbeddedForm); the
/// derive knows the type's shape, this module knows the columns. A hand-written
/// impl is possible and is what the derive expands to — see the derive's
/// documentation for the exact shape.
pub trait EmbeddedForm: Sized {
    /// Write this value's leaves into `out`, under the columns the app schema
    /// resolves for `parent`.
    ///
    /// The **active variant's** leaves are written for an enum, plus its
    /// discriminant: a value has one variant, and the form must say which.
    fn write_form<M>(&self, cx: &Cx, parent: Path<M, Self>, out: &mut HashMap<String, String>)
    where
        M: toasty::schema::Model;

    /// Read a value back from a submission.
    ///
    /// An embedded enum takes its variant from the discriminant key, falling
    /// back to the first variant when the submission does not carry one (a
    /// hand-written POST, or the create form before a variant control exists).
    /// Payload emptiness is never consulted.
    fn read_form<M>(cx: &Cx, parent: Path<M, Self>, values: &HashMap<String, String>) -> Self
    where
        M: toasty::schema::Model;
}

/// The form key one leaf occupies — its flattened storage column.
///
/// Generated code calls this once per leaf; an app writing a codec by hand uses
/// it the same way. It is the single-leaf half of [`FieldResolver::resolve`].
pub fn leaf_key<M, T>(cx: &Cx, path: impl Into<Path<M, T>>) -> String
where
    M: toasty::schema::Model,
{
    FieldResolver::from_cx(cx).resolve(path.into()).name
}

/// An embedded enum's discriminant column and variant values (GH #191).
#[derive(Debug, Clone)]
pub struct EnumSpec {
    discriminant: String,
    variants: Vec<(String, String)>,
}

impl EnumSpec {
    /// The discriminant column the form carries (`kind`).
    pub fn discriminant(&self) -> &str {
        &self.discriminant
    }

    /// `(Rust variant name, stored discriminant text)`, in declaration order.
    pub fn variants(&self) -> &[(String, String)] {
        &self.variants
    }

    /// The discriminant text of `variant`.
    pub fn value_of(&self, variant: &str) -> Option<&str> {
        self.variants
            .iter()
            .find(|(name, _)| name == variant)
            .map(|(_, value)| value.as_str())
    }

    /// The variant a submission selects, if it carries a known discriminant.
    pub fn variant_of(&self, submitted: &str) -> Option<&str> {
        self.variants
            .iter()
            .find(|(_, value)| value == submitted)
            .map(|(name, _)| name.as_str())
    }
}

/// The discriminant column and variant values for the embedded **enum** at
/// `parent`, or `None` when the value is an embedded struct.
///
/// Panics when `parent` names no embedded value at all: every caller is a
/// declaration (`#[derive(EmbeddedForm)]` on an enum, or a form built from
/// one), and a lens that resolves to nothing is a wiring bug, not user input
/// (the GH #100 policy).
pub fn enum_spec<M, T>(cx: &Cx, parent: impl Into<Path<M, T>>) -> Option<EnumSpec>
where
    M: toasty::schema::Model,
{
    let spec = FieldResolver::from_cx(cx)
        .resolve_embedded_value(parent.into())
        .unwrap_or_else(|| {
            panic!(
                "{} is not an embedded value in this request's app schema: a value binding \
                 needs an embedded struct or enum field and a `Db` in context (GH #191)",
                std::any::type_name::<T>()
            )
        });
    spec.enum_spec.map(|spec| EnumSpec {
        discriminant: spec.discriminant,
        variants: spec.variants,
    })
}

/// Every form key the value at `parent` occupies: its leaf columns, plus the
/// discriminant for an enum.
pub fn form_keys<M, T>(cx: &Cx, parent: impl Into<Path<M, T>>) -> Vec<String>
where
    M: toasty::schema::Model,
{
    let spec = FieldResolver::from_cx(cx)
        .resolve_embedded_value(parent.into())
        .unwrap_or_else(|| {
            panic!(
                "{} is not an embedded value in this request's app schema (GH #191)",
                std::any::type_name::<T>()
            )
        });
    let mut keys = spec.columns;
    if let Some(enum_spec) = spec.enum_spec {
        keys.push(enum_spec.discriminant);
    }
    keys
}

/// Whether a submission carries any key of the value at `parent` (GH #191).
///
/// The update half of the presence rule (GH #89): a submit that never mentions
/// this value leaves it alone, and "mentions" is decided by the columns the
/// schema resolves rather than by a name the app spells.
pub fn submitted<M, T>(
    cx: &Cx,
    parent: impl Into<Path<M, T>>,
    values: &HashMap<String, String>,
) -> bool
where
    M: toasty::schema::Model,
{
    form_keys(cx, parent)
        .iter()
        .any(|key| values.contains_key(key))
}

/// The hidden control that carries an embedded enum's discriminant (GH #191).
///
/// `None` for an embedded struct, which has no variant to name. The control is
/// a hidden `TextInput`, so it rides the existing value map, validation, and
/// re-render paths instead of adding a field kind.
pub fn discriminant_input<M, T>(cx: &Cx, parent: impl Into<Path<M, T>>) -> Option<TextInput>
where
    M: toasty::schema::Model,
{
    enum_spec(cx, parent).map(|spec| TextInput::hidden(spec.discriminant().to_string()))
}

/// Write the typed `value` into the form map, under the columns the schema
/// resolves for `parent` (GH #191).
pub fn write_embedded<M, T>(
    cx: &Cx,
    parent: impl Into<Path<M, T>>,
    value: &T,
    out: &mut HashMap<String, String>,
) where
    M: toasty::schema::Model,
    T: EmbeddedForm,
{
    value.write_form(cx, parent.into(), out);
}

/// Read the typed value back from a submission (GH #191).
pub fn read_embedded<M, T>(
    cx: &Cx,
    parent: impl Into<Path<M, T>>,
    values: &HashMap<String, String>,
) -> T
where
    M: toasty::schema::Model,
    T: EmbeddedForm,
{
    T::read_form(cx, parent.into(), values)
}

/// Read one leaf out of a submission, by its resolved key (GH #191).
///
/// Trimmed; an absent or empty value is the type's `Default`, which is the
/// panel's rule for a typed column with no spelling for "no value" (GH #192) —
/// an optional typed leaf left blank reaches its record fn as that default.
///
/// A value the type **cannot** parse panics instead: typed controls refuse
/// those inline before a record fn runs, so reaching here with one means the
/// form was bypassed, and a silent default is exactly the bug GH #192 fixed.
/// The panic names the column and the type's own parse error.
pub fn parse_leaf<T>(key: &str, values: &HashMap<String, String>) -> T
where
    T: std::str::FromStr + Default,
    T::Err: std::fmt::Display,
{
    let Some(raw) = values.get(key) else {
        return T::default();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return T::default();
    }
    match trimmed.parse::<T>() {
        Ok(value) => value,
        Err(error) => panic!("`{trimmed}` is not a valid value for `{key}`: {error} (GH #191)"),
    }
}
