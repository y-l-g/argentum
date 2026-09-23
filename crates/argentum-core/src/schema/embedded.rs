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
//! ```text
//! let publication = if !field(values, "publication_canonical_url").is_empty() {
//!     Publication::Published { published_at: timestamp, canonical_url: … }
//! } else if … // and so on, per app, per type
//! ```
//!
//! That inference is the bug this module removes. An embedded enum *has* a
//! discriminant column — Toasty stores the variant there — so the form carries
//! the discriminant explicitly, hydration writes the stored variant, and an
//! edit names the variant it means.
//!
//! Payloads still decide **one** case, and only when the submission carries no
//! discriminant at all: the create form has no stored variant to hydrate, so a
//! payload of the author's own selects the variant (a `#[shared(..)]` column
//! never does, because it belongs to several). That is the panel's pre-#191
//! rule, reimplemented from the keys the schema resolves rather than remembered
//! column names — and a discriminant the submission *does* name always wins,
//! with an unknown one refused loudly rather than read as something else.
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
//!   so there is no per-field binding, and the walk refuses rather than hand one
//!   column back for several fields. Leaf binding of a document still works
//!   (GH #185).
//! - A relation inside an embedded value: relations are not stored in the row.
//! - An embedded enum nested **inside an enum variant**: value resolution starts
//!   at a model root, and a variant-rooted path addresses one variant's leaf
//!   (which leaf binding does handle). Nesting inside *structs* works at any
//!   depth.
//! - The variant **control** is a `Select` over the discriminant column
//!   ([`discriminant_select`]), one option per variant — submitting the value
//!   the column stores, reading as the variant's name — and each variant's
//!   payload renders inside a `Group` marked with that variant's value, so the
//!   client can show the chosen variant alone (GH #191). The grouping is the
//!   derive's, the marker is
//!   [`Group::variant`](crate::schema::Group::variant), and the toggle is
//!   `assets/variant.js`; with JavaScript off every group renders, which is
//!   what the panel has always done.
//!
//! # What a derived control declares
//!
//! Every leaf under an embedded step is **not required** by default — the
//! resolver reports `nullable=true` by binding policy, since only the matching
//! variant writes a variant payload column — which is what the hand-written
//! forms spelled `.optional()` for. That is the binding default, not a storage
//! fact: the flattened column of a required embedded struct is `NOT NULL`. A
//! leaf that must be present says so on the field's type or the app marks the
//! control in its own layout.

use std::collections::HashMap;

use toasty::stmt::Path;
use topcoat::context::Cx;

use crate::schema::Select;
use crate::schema::lenses::FieldResolver;

/// The resolver for this request, with the one failure a value binding cannot
/// fall back from: no app schema means no columns.
fn resolver(cx: &Cx) -> FieldResolver<'_> {
    let resolver = FieldResolver::from_cx(cx);
    assert!(
        resolver.has_schema(),
        "an embedded value needs the app schema: put a `Db` in the context (GH #191)"
    );
    resolver
}

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
    /// An embedded enum takes its variant from the discriminant key; a
    /// submission that names one it does not declare is refused loudly, and one
    /// that carries no discriminant at all falls back to the first variant
    /// whose own payload was submitted (the create form, a hand-written POST),
    /// else the first variant. That fallback is the panel's pre-#191 rule,
    /// reimplemented from the keys the schema resolves rather than remembered
    /// column names.
    fn read_form<M>(cx: &Cx, parent: Path<M, Self>, values: &HashMap<String, String>) -> Self
    where
        M: toasty::schema::Model;

    /// Whether a submission mentions any key of this value (GH #191).
    ///
    /// The presence rule (GH #89) at the *value* level: an update that never
    /// mentions a value leaves it alone, and "mentions" is decided by the keys
    /// the schema resolves — the discriminant included, at every nesting
    /// level. Generated code answers it, so a nested value composes; the parent
    /// path of a field inside an enum variant is variant-rooted, which leaf
    /// resolution handles ([`leaf_key`]) where whole-value resolution does not.
    fn any_present<M>(cx: &Cx, parent: Path<M, Self>, values: &HashMap<String, String>) -> bool
    where
        M: toasty::schema::Model;
}

/// The form key one leaf occupies — its flattened storage column.
///
/// Generated code calls this once per leaf; an app writing a codec by hand uses
/// it the same way. It is the single-leaf half of `FieldResolver::resolve`.
pub fn leaf_key<M, T>(cx: &Cx, path: impl Into<Path<M, T>>) -> String
where
    M: toasty::schema::Model,
{
    resolver(cx).resolve(path.into()).name
}

/// An embedded enum's discriminant column and variants (GH #191).
///
/// A variant carries two things, and they are not interchangeable: the **value**
/// its discriminant column stores (`2`, or a string discriminant's own text),
/// which is the form's transport, and the **name** a person reads (`Published`),
/// which is the control's label. [`discriminant_select`] submits the first and
/// shows the second.
///
/// The name is the schema's — Toasty keeps a `Name` as word parts, so it
/// survives in both spellings — but it is humanized into sentence case here
/// (`In progress`), exactly as a derived field label is, because a label is the
/// one place an identifier becomes prose. It is a *label*, never a handle: the
/// normalization is lossy in the other direction (`OK` reads `Ok`), so code
/// addresses variants by **declaration index**, the same handle the schema
/// itself uses (`VariantId { index }`) and the one a generated codec can rely
/// on (the app schema lists variants in the order the Rust enum declares them).
#[derive(Debug, Clone)]
pub struct EnumSpec {
    discriminant: String,
    variants: Vec<Variant>,
}

/// One variant of an embedded enum: what it stores, and what it is called.
#[derive(Debug, Clone)]
struct Variant {
    /// The discriminant text this variant stores (`2`).
    value: String,
    /// The name it is labelled with, in sentence case (`Published`).
    name: String,
}

impl EnumSpec {
    pub(crate) fn new(discriminant: String, variants: Vec<(String, String)>) -> Self {
        Self {
            discriminant,
            variants: variants
                .into_iter()
                .map(|(value, name)| Variant { value, name })
                .collect(),
        }
    }

    /// The discriminant column the form carries (`kind`).
    pub fn discriminant(&self) -> &str {
        &self.discriminant
    }

    /// The discriminant text variant `index` stores, if the enum declares it.
    pub fn value_of_index(&self, index: usize) -> Option<&str> {
        self.variants
            .get(index)
            .map(|variant| variant.value.as_str())
    }

    /// The name variant `index` is labelled with, if the enum declares it.
    ///
    /// The label half of [`Self::value_of_index`]: the control shows this and
    /// submits that.
    pub fn name_of_index(&self, index: usize) -> Option<&str> {
        self.variants
            .get(index)
            .map(|variant| variant.name.as_str())
    }

    /// The index of the variant a submission names, if it names a known one.
    pub fn index_of(&self, submitted: &str) -> Option<usize> {
        self.variants
            .iter()
            .position(|variant| variant.value == submitted)
    }

    /// How many variants the enum declares.
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// Whether the enum declares no variants at all.
    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
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
    let spec = resolver(cx)
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
/// discriminant of every enum it contains.
///
/// Internal to [`submitted`]: a value answers presence for itself through
/// [`EmbeddedForm::any_present`], and this is the flat-map answer for a whole
/// value at a model-rooted path.
fn form_keys<M, T>(cx: &Cx, parent: impl Into<Path<M, T>>) -> Vec<String>
where
    M: toasty::schema::Model,
{
    let spec = resolver(cx)
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

/// The variant control for an embedded enum (GH #191): a `Select` over the
/// discriminant column, one option per variant the app schema declares.
///
/// Each option **submits the variant's stored discriminant** and **reads as its
/// name** (`Published`): the form's transport stays the value the column holds,
/// while the person choosing sees which state they are picking. A control
/// offering `1` / `2` / `3` would be the hidden input made clickable.
///
/// `None` for an embedded struct, which has no variant to choose. The control
/// is a `Select` rather than a hidden input so the variant is **choosable** —
/// on create there is no stored variant to hydrate, and on edit the stored one
/// must be changeable — and it is the driver the derived variant groups follow:
/// it renders `data-variant-select`, and each group carries `data-variant-of`
/// (this column) plus `data-variant` (the value this select offers for it), so
/// `variant.js` can show only the chosen variant's payload.
///
/// It is not required: an empty submit is "no variant named", which
/// [`EmbeddedForm::read_form`] answers with its own payload fallback, so
/// refusing it would make that fallback unreachable from the form.
pub fn discriminant_select<M, T>(cx: &Cx, parent: impl Into<Path<M, T>>) -> Option<Select>
where
    M: toasty::schema::Model,
{
    enum_spec(cx, parent).map(|spec| {
        let options = (0..spec.len())
            .filter_map(|index| {
                Some((
                    spec.value_of_index(index)?.to_string(),
                    spec.name_of_index(index)?.to_string(),
                ))
            })
            .collect();
        Select::named(spec.discriminant()).options_with_labels(options)
    })
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
