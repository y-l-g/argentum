//! Field lenses — typed Toasty paths and their app-level metadata.
//!
//! Base bridge layer (with `pk`): these two modules are the only schema
//! modules that name `toasty_core` (upstream #114/#183), so upstream
//! churn has one blast radius. Everything above reads lenses through
//! `FieldLens` and the helpers here.
//!
//! [`lens_field`] hands callers the built `app::Field`, so name, label,
//! nullability, storage name, `FieldTy`, `auto`, and `constraints` all come
//! from one walk. [`lens_field_unique`] covers the one property `Field` does
//! not carry, because Toasty keeps uniqueness on the model's index list.
//!
//! [`FieldResolver`] is the schema-aware walk (GH #185): with the app schema
//! in hand an embedded path resolves to its **flattened storage column**, so
//! `TextInput::r#for_post(...)`-style bindings can reach a field inside an
//! embedded struct or a `#[document]`. Without a schema the single-segment
//! rule (GH #100) still applies — the owned `app::Model` cannot see embedded
//! models, which is exactly what the schema adds.

use toasty_core::schema::app::{FieldTy, Model};
use toasty_core::stmt::PathRoot;
use topcoat::context::Cx;

/// Spec alias — ADR-0001 typed lens. Currently uses `toasty::stmt::Path` directly;
/// a richer `FieldLens` trait will replace this alias if Toasty exposes the
/// metadata walk directly (see GH #11, upstream issue #183).
pub type FieldLens<M, T> = toasty::stmt::Path<M, T>;

/// The storage column an embedded path flattens to, plus the metadata field
/// constructors need.
///
/// Owned because the schema is borrowed from the request context for the length
/// of one call and no borrow of it can be held by a `Schema`.
#[derive(Debug, Clone)]
pub(crate) struct LeafField {
    /// The flattened storage column (`application_seo_title`), or the field's
    /// own name for a single-segment path.
    pub(crate) name: String,
    pub(crate) label: String,
    /// Whether the leaf column is nullable. An embedded step makes every
    /// column under it storage-nullable — only the matching enum variant
    /// writes a value — so an embedded leaf is never required by default.
    pub(crate) nullable: bool,
}

/// The app schema for this request, or `None` when no `Db` is in context.
///
/// It is the same `app::Schema` the `Db` was compiled from — `Db::schema()` is
/// public and its `app` field carries the embedded models the owned
/// `Model::schema()` cannot see — so the request path can resolve an embedded
/// lens without opening a connection.
///
/// Borrowed, never cloned, and optional: a bare `CxTestBuilder` has no `Db`, and
/// then the single-segment rule (GH #100) applies, so a schema-less test fails
/// loudly on a traversal lens rather than silently binding the first segment.
fn request_schema(cx: &Cx) -> Option<&toasty_core::schema::app::Schema> {
    topcoat::context::try_app_context::<toasty::Db>(cx).map(|db| &db.schema().app)
}

/// Walks a lens path against the app schema, resolving embedded steps.
///
/// Kept as a struct rather than free functions so the schema borrow has one
/// owner and the constructors can share one resolution between the name, the
/// label, and the uniqueness check.
pub(crate) struct FieldResolver<'a> {
    schema: Option<&'a toasty_core::schema::app::Schema>,
}

impl<'a> FieldResolver<'a> {
    /// Build the resolver from the app schema this request carries, if any.
    pub(crate) fn from_cx(cx: &'a Cx) -> Self {
        Self {
            schema: request_schema(cx),
        }
    }

    /// Resolve a lens to its leaf field.
    ///
    /// With a schema this walks the whole projection, so an embedded step lands
    /// on the flattened column. Without one it falls back to the single-segment
    /// rule and panics on a traversal lens — loudly, because silently binding
    /// the first segment misbinds in release (GH #100).
    pub(crate) fn resolve<M, T>(&self, path: FieldLens<M, T>) -> LeafField
    where
        M: toasty::schema::Model,
    {
        let model = M::schema();
        let core_path: toasty_core::stmt::Path = path.into();
        let segments = core_path.projection.as_slice().len();
        // A variant root is an embedded-enum payload accessor *whatever* its
        // projection length: the generated accessor rebases onto the variant, so
        // the steps are variant-local and a single-step projection is the
        // payload field. Only a model root with one step is a plain field.
        let is_embedded_path = segments > 1 || matches!(core_path.root, PathRoot::Variant { .. });
        if is_embedded_path && let Some(schema) = self.schema {
            return self
                .walk_embedded(schema, &core_path, &model)
                .unwrap_or_else(|| {
                    panic!(
                        "lens path {projection:?} does not resolve to a single column for {}: \
                         only embedded steps (embedded structs, enum variant fields, and \
                         `#[document]` fields) can be bound, not relation hops, and only when \
                         the model is in the app schema this request carries (GH #185)",
                        std::any::type_name::<M>(),
                        projection = core_path.projection.as_slice(),
                    )
                });
        }
        // No schema: the single-segment rule is all an owned `app::Model` can
        // answer, so resolve the first step against `ModelRoot.fields` and let
        // `require_single_segment` reject a traversal lens (GH #100).
        require_single_segment(&core_path, "lens");
        let idx = core_path
            .projection
            .as_slice()
            .first()
            .copied()
            .expect("field lens must have a projection");
        let field = model
            .as_root_unwrap()
            .fields
            .get(idx)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "field index {idx} out of bounds for {}",
                    std::any::type_name::<M>()
                )
            });
        LeafField {
            name: field.name.app_unwrap().to_string(),
            label: lens_label(&field),
            nullable: field.nullable(),
        }
    }

    /// Walk a lens path to the column it names.
    ///
    /// Two roots reach here:
    ///
    /// - a **model** root, for a plain path (`seo.title`) — the steps walk the
    ///   model's fields;
    /// - a **variant** root, for an enum payload accessor
    ///   (`publication.published().published_at()`). The generated accessor
    ///   rebases onto the variant, so the root carries the parent path *to the
    ///   enum field* while the projection's steps are **variant-local**.
    ///
    /// This mirrors `app::Schema::resolve`'s traversal (upstream
    /// `schema/app/schema.rs`) rather than calling it, because that resolver
    /// returns only the leaf `Field` — whose `name` is its own (`title`), not
    /// the flattened column (`seo_title`). The name is built from the steps, so
    /// the walk has to keep them.
    ///
    /// Only embedded steps are followed. A relation hop (a `BelongsTo` /
    /// `Has` / `Via` step) yields `None`: this walk exists for embedded binding
    /// (GH #185), and binding anything else here would reintroduce the misbind
    /// GH #100 guards against.
    ///
    /// Column naming was measured, not assumed:
    /// - an embedded struct prefixes its field's name (`seo_title`,
    ///   `media_video_poster_credit_author`);
    /// - an enum payload adds the payload field's name, and a `#[shared]`
    ///   payload adds its **shared identifier** instead (`publication_timestamp`
    ///   for all three variants) — never the variant's name, which lives in the
    ///   discriminant column;
    /// - a `#[document]` collapses to **one** column named after the document
    ///   field, so the walk stops there.
    fn walk_embedded(
        &self,
        schema: &'a toasty_core::schema::app::Schema,
        path: &toasty_core::stmt::Path,
        owner: &toasty::schema::app::Model,
    ) -> Option<LeafField> {
        match &path.root {
            PathRoot::Model(id) => {
                let root = schema.get_model(*id)?.as_root()?;
                // The accessor's `ModelId` and the schema's may come from
                // different `models!(..)` expansions, so an id can name a
                // *different* model here. Verify identity by root model name
                // before trusting the index: binding another model's column is
                // the misbind GH #100 exists to prevent.
                if root.name.upper_camel_case() != owner.as_root()?.name.upper_camel_case() {
                    return None;
                }
                let [first, rest @ ..] = path.projection.as_slice() else {
                    return None;
                };
                let first_field = root.fields.get(*first)?;
                walk_steps(schema, first_field, rest).map(LeafField::from)
            }
            // An enum payload: resolve the parent path (it ends at the enum
            // field), then walk the variant's own fields.
            PathRoot::Variant { parent, variant_id } => {
                let root = schema.get_model(parent.root.as_model_unwrap())?.as_root()?;
                let [first, rest @ ..] = parent.projection.as_slice() else {
                    return None;
                };
                let walked = walk_steps(schema, root.fields.get(*first)?, rest)?;
                let FieldTy::Embedded(embedded) = &walked.field.ty else {
                    return None;
                };
                let Some(Model::EmbeddedEnum(e)) = schema.get_model(embedded.target) else {
                    return None;
                };
                e.variants.get(variant_id.index)?;
                // `variant_id` may come from a different expansion than this
                // schema's enum, so a variant that declares no fields here is
                // not the variant we resolved — refuse rather than bind the
                // first payload column we find.
                e.fields
                    .iter()
                    .find(|f| f.variant.as_ref() == Some(variant_id))?;
                let variant = Variant {
                    enum_model: e,
                    id: variant_id,
                };
                let [step, tail @ ..] = path.projection.as_slice() else {
                    return None;
                };
                let field = variant.field(*step)?;
                let mut prefix = walked.prefix;
                prefix.push(column_segment(field));
                walk_steps_with_prefix(schema, field, tail, prefix).map(LeafField::from)
            }
        }
    }
}

/// One variant's fields, resolved against a variant id that may originate from
/// a different `models!(..)` expansion than this schema's enum.
struct Variant<'a> {
    enum_model: &'a toasty_core::schema::app::EmbeddedEnum,
    id: &'a toasty_core::schema::app::VariantId,
}

impl<'a> Variant<'a> {
    /// The `step`th payload field of this variant, if the variant owns one.
    fn field(&self, step: usize) -> Option<&'a toasty_core::schema::app::Field> {
        self.enum_model
            .fields
            .iter()
            .filter(|f| f.variant.as_ref() == Some(self.id))
            .nth(step)
    }
}

/// Walk the steps after `field`, accumulating the flattened column name.
fn walk_steps<'a>(
    schema: &'a toasty_core::schema::app::Schema,
    field: &'a toasty_core::schema::app::Field,
    steps: &[usize],
) -> Option<Walked<'a>> {
    let prefix = vec![column_segment(field)];
    walk_steps_with_prefix(schema, field, steps, prefix)
}

/// The recursive core: descend `steps`, extending `prefix` as the column nests.
fn walk_steps_with_prefix<'a>(
    schema: &'a toasty_core::schema::app::Schema,
    field: &'a toasty_core::schema::app::Field,
    steps: &[usize],
    prefix: Vec<String>,
) -> Option<Walked<'a>> {
    // A `#[document]` field is a *primitive* whose storage is a `Model` (probed:
    // `FieldPrimitive { ty: Model(id) }`), not an `Embedded`. Its sub-fields
    // share the one column named after the document field, so the walk stops:
    // `post_stats.word_count` binds the column `post_stats`.
    if let FieldTy::Primitive(primitive) = &field.ty
        && matches!(primitive.ty, toasty_core::stmt::Type::Model(_))
    {
        return Some(Walked { prefix, field });
    }
    let Some((step, rest)) = steps.split_first() else {
        // The path ended: this field is the leaf.
        return Some(Walked { prefix, field });
    };
    let FieldTy::Embedded(embedded) = &field.ty else {
        // A real primitive is a leaf with steps left over, and a relation is
        // out of scope by design.
        return None;
    };
    match schema.get_model(embedded.target)? {
        Model::EmbeddedStruct(s) => {
            let next = s.fields.get(*step)?;
            let mut prefix = prefix;
            prefix.push(column_segment(next));
            walk_steps_with_prefix(schema, next, rest, prefix)
        }
        Model::EmbeddedEnum(e) => {
            // A variant step is a gate, not a name.
            e.variants.get(*step)?;
            let (field_step, tail) = rest.split_first()?;
            // Enum-global field index, the form a model-rooted payload path
            // produces.
            let next = e.fields.get(*field_step)?;
            let mut prefix = prefix;
            prefix.push(column_segment(next));
            walk_steps_with_prefix(schema, next, tail, prefix)
        }
        _ => None,
    }
}

/// The name a field contributes to its flattened column.
///
/// A `#[shared(<ident>)]` field contributes the **identifier**, not its own
/// name: that is what coalesces several variants' fields into one column, and
/// why `Publication::Scheduled.scheduled_at` and `Publication::Published
/// .published_at` both live in `publication_timestamp`.
fn column_segment(field: &toasty_core::schema::app::Field) -> String {
    match &field.shared {
        // `Name`'s parts are already the lowercase words, so joining them is
        // the snake_case spelling of the identifier.
        Some(shared) => shared.parts.join("_"),
        None => field.name.app_unwrap().to_string(),
    }
}

/// A walked path: the accumulated column prefix and the leaf field it reaches.
struct Walked<'a> {
    prefix: Vec<String>,
    field: &'a toasty_core::schema::app::Field,
}

impl From<Walked<'_>> for LeafField {
    fn from(walked: Walked<'_>) -> Self {
        LeafField {
            name: walked.prefix.join("_"),
            label: capitalize(&walked.field.name.app_unwrap().replace('_', " ")),
            // Every column under an embedded step is storage-nullable: only
            // the matching variant writes a value.
            nullable: true,
        }
    }
}

/// Resolve a typed lens to the app-level [`Field`] behind it.
///
/// The single walk over `Path → toasty_core::stmt::Path → projection`, reading
/// off the model's built schema (upstream issues #114/#183). Callers read
/// name, label, nullability, storage name, `FieldTy`, `auto`, and `constraints`
/// off the result; uniqueness is the one property a `Field` cannot answer, so
/// it has [`lens_field_unique`] of its own.
///
/// `model` is the owning `Model::schema()`, passed in so one form-input
/// construction resolves the schema once (never per row) and shares it with
/// [`lens_field_unique`]. The returned `Field` is owned because
/// `Model::schema()` builds the model by value with no cache, so no borrow of
/// it can escape — but only the one field is cloned.
///
/// Traversal lenses are rejected (GH #100): a multi-step path has no single
/// field name, and silently binding its first segment misbinds in release.
/// Use [`FieldResolver`] (GH #185) to bind an embedded path instead.
pub(crate) fn lens_field<M, T>(
    path: FieldLens<M, T>,
    model: &toasty::schema::app::Model,
) -> toasty::schema::app::Field
where
    M: toasty::schema::Model,
{
    let core_path: toasty_core::stmt::Path = path.into();
    require_single_segment(&core_path, "lens");
    let idx = core_path
        .projection
        .as_slice()
        .first()
        .copied()
        .expect("field lens must have a projection");
    model
        .as_root_unwrap()
        .fields
        .get(idx)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "field index {idx} out of bounds for {}",
                std::any::type_name::<M>()
            )
        })
}

/// The capitalized label for a field's app-level name.
pub(crate) fn lens_label(field: &toasty::schema::app::Field) -> String {
    capitalize(field.name.app_unwrap())
}

/// Whether `field` is backed by a single-field unique index.
///
/// Uniqueness is not a property of a `Field`: Toasty stores it on the model's
/// index list, so this needs the owning `ModelRoot` too. Only a *single-field*
/// unique index counts — the components of a composite `#[unique(a, b)]` are
/// not unique on their own — and the primary key is excluded, since a key
/// column is unique by construction rather than by a declared constraint.
///
/// This reports declared schema uniqueness, not a global uniqueness guarantee:
/// SQL permits multiple `NULL`s in a unique index, and enum-variant columns are
/// storage-nullable, so a nullable unique field can still repeat. That is the
/// same caveat the app-side pre-check has always carried (`panel/forms.rs`).
pub(crate) fn lens_field_unique(
    field: &toasty::schema::app::Field,
    model: &toasty::schema::app::ModelRoot,
) -> bool {
    model.indices.iter().any(|index| {
        index.unique
            && !index.primary_key
            && index.fields.len() == 1
            && index.fields[0].field == field.id
    })
}

/// Panic unless a lens path addresses exactly one field (GH #100).
///
/// A traversal lens (relation hops, embedded steps) has no single field name,
/// nullability, or uniqueness — silently binding its first segment misbinds in
/// release, so every lens helper rejects multi-segment paths loudly instead.
pub(crate) fn require_single_segment(path: &toasty_core::stmt::Path, what: &str) {
    assert_eq!(
        path.projection.as_slice().len(),
        1,
        "{what} requires a single-field lens, got a {}-segment traversal path (GH #100)",
        path.projection.as_slice().len()
    );
}

pub(crate) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        #[unique]
        email: String,
    }

    #[test]
    fn single_segment_lens_passes_traversal_panics() {
        use toasty::schema::Model;
        let single = toasty_core::stmt::Path::field(DummyUser::id(), 0);
        require_single_segment(&single, "lens");
        let mut two = toasty_core::stmt::Path::field(DummyUser::id(), 0);
        two.chain(&toasty_core::stmt::Path::field(DummyUser::id(), 1));
        let result = std::panic::catch_unwind(|| require_single_segment(&two, "lens"));
        assert!(result.is_err(), "traversal lens must panic, not misbind");
    }

    /// The lens resolves to the field it names, labels it, and reports the
    /// nullability the required-default reads.
    #[test]
    fn lens_field_resolves_name_label_and_nullability() {
        use toasty::schema::Model;
        let model = DummyUser::schema();

        let email = lens_field(DummyUser::fields().email(), &model);
        assert_eq!(email.name.app_unwrap(), "email");
        assert_eq!(lens_label(&email), "Email");
        assert!(!email.nullable());
        assert_eq!(email.name.storage_name(), Some("email"));

        let name = lens_field(DummyUser::fields().name(), &model);
        assert_eq!(name.name.app_unwrap(), "name");
        assert_eq!(lens_label(&name), "Name");
    }

    /// `#[unique]` lives on the model's index list, not the field, so
    /// uniqueness is a separate lookup — and a bare field is not unique.
    #[test]
    fn lens_field_unique_reads_the_model_index_list() {
        use toasty::schema::Model;
        let model = DummyUser::schema();
        let root = model.as_root_unwrap();

        let email = lens_field(DummyUser::fields().email(), &model);
        assert!(
            lens_field_unique(&email, root),
            "#[unique] on email must surface as a single-field unique index"
        );

        let name = lens_field(DummyUser::fields().name(), &model);
        assert!(
            !lens_field_unique(&name, root),
            "a field with no unique index must not report unique"
        );

        let id = lens_field(DummyUser::fields().id(), &model);
        assert!(
            !lens_field_unique(&id, root),
            "the primary key is unique by construction, not by declared constraint"
        );
    }
}
