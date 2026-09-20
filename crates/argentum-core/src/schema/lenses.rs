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

/// The compiled schema for this request, or `None` when no `Db` is in context.
///
/// `Db::schema()` is public and gives all three halves the walk needs: `.app`
/// carries the embedded models the owned `Model::schema()` cannot see, and
/// `.mapping` records which physical column each field resolves to. So the
/// request path can bind an embedded lens without opening a connection.
///
/// Borrowed, never cloned, and optional: a bare `CxTestBuilder` has no `Db`, and
/// then the single-segment rule (GH #100) applies, so a schema-less test fails
/// loudly on a traversal lens rather than silently binding the first segment.
fn request_schema(cx: &Cx) -> Option<&toasty_core::Schema> {
    topcoat::context::try_app_context::<toasty::Db>(cx).map(|db| &**db.schema())
}

/// Walks a lens path against the app schema, resolving embedded steps.
///
/// Kept as a struct rather than free functions so the schema borrow has one
/// owner and the constructors can share one resolution between the name, the
/// label, and the uniqueness check.
pub(crate) struct FieldResolver<'a> {
    /// The **compiled** schema: its `.app` half resolves the path, its
    /// `.mapping` half names the column, its `.db` half holds the name.
    schema: Option<&'a toasty_core::Schema>,
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

    /// Walk a lens path to the physical column it names.
    ///
    /// Two sources, each authoritative for one thing:
    ///
    /// - the **app schema** drives the traversal, because it is the only side
    ///   that knows a field is a `#[document]` — a *primitive* whose storage is
    ///   a model, so its inner fields collapse into the one column named after
    ///   it, and a path through one has steps left over on arrival;
    /// - the **compiled mapping** names the column. `Db::schema().mapping`
    ///   records, per model, which column every field resolves to — flattened
    ///   embedded structs, enum discriminant and payload columns, shared
    ///   columns, and a document's single column alike — so this reads the name
    ///   off `db::Table` rather than re-deriving Toasty's naming rules. An
    ///   earlier revision accumulated names from `app::Field.name` and had to
    ///   encode those rules itself.
    ///
    /// Probed against the pinned rev the two agree for every reachable shape,
    /// including the cases that are easy to get wrong: `#[shared(..)]` payloads
    /// collapse onto one column (`publication_timestamp`), and a `#[document]`
    /// is a single `Primitive` column named after the field (`stats`), not one
    /// column per inner field.
    ///
    /// Two roots reach here:
    ///
    /// - a **model** root, for a plain path (`seo.title`);
    /// - a **variant** root, for an enum payload accessor
    ///   (`media.video().poster().url()`): the generated accessor rebases onto
    ///   the variant, so the root carries the parent path *to the enum field*
    ///   while the projection's steps are **variant-local**.
    ///
    /// Only embedded steps are followed. A relation hop yields `None`: this
    /// exists for embedded binding (GH #185), and binding anything else here
    /// would reintroduce the misbind GH #100 guards against.
    fn walk_embedded(
        &self,
        schema: &'a toasty_core::Schema,
        path: &toasty_core::stmt::Path,
        owner: &toasty::schema::app::Model,
    ) -> Option<LeafField> {
        match &path.root {
            PathRoot::Model(id) => {
                let root = schema.app.get_model(*id)?.as_root()?;
                // The accessor's `ModelId` and the schema's may come from
                // different `models!(..)` expansions, so an id can name a
                // *different* model here. Verify identity by root model name
                // before trusting any index: binding another model's column is
                // the misbind GH #100 exists to prevent.
                if root.name.upper_camel_case() != owner.as_root()?.name.upper_camel_case() {
                    return None;
                }
                let model = schema.mapping.models.get(&root.id)?;
                descend(
                    schema,
                    &root.fields,
                    &model.fields,
                    path.projection.as_slice(),
                )
            }
            // An enum payload: resolve the parent path (it ends at the enum
            // field), then descend into the variant the accessor selected.
            PathRoot::Variant { parent, variant_id } => {
                let root = schema
                    .app
                    .get_model(parent.root.as_model_unwrap())?
                    .as_root()?;
                let model = schema.mapping.models.get(&root.id)?;
                let parent_steps = parent.projection.as_slice();
                // The app side supplies the enum's payload field list, the
                // mapping side its per-variant column mappings.
                let app_field = app_field_at(schema, &root.fields, parent_steps)?;
                let Some(toasty::schema::app::Model::EmbeddedEnum(e)) =
                    app_embedded(schema, app_field)
                else {
                    return None;
                };
                let MappingField::Enum(me) = mapping_field_at(&model.fields, parent_steps)? else {
                    return None;
                };
                let variant = me.variants.get(variant_id.index)?;
                let payloads: Vec<_> = e.variant_fields(variant_id.index).collect();
                descend(
                    schema,
                    &payloads,
                    &variant.fields,
                    path.projection.as_slice(),
                )
            }
        }
    }
}

/// A `mapping::Field`, aliased so the traversal signatures stay readable.
type MappingField = toasty_core::schema::mapping::Field;

/// Walk the path down to the column that stores it.
///
/// `app_fields` and `mapping_fields` are indexed by the same field index, so
/// every step reads both: the app side decides *whether to descend*, the mapping
/// side names *the column*. Neither alone is enough — the mapping cannot say a
/// field is a `#[document]`, and the app schema cannot say which column a leaf
/// occupies without re-deriving Toasty's naming.
fn descend<F>(
    schema: &toasty_core::Schema,
    app_fields: &[F],
    mapping_fields: &[MappingField],
    steps: &[usize],
) -> Option<LeafField>
where
    F: std::borrow::Borrow<toasty::schema::app::Field>,
{
    let (first, rest) = steps.split_first()?;
    let app_field: &toasty::schema::app::Field = app_fields.get(*first)?.borrow();
    let mapping_field = mapping_fields.get(*first)?;
    // A `#[document]`'s inner fields share its one column, so reaching the
    // document is reaching the leaf, however many steps remain.
    if rest.is_empty() || is_document(app_field) {
        return column_of(schema, mapping_field);
    }
    let toasty::schema::app::FieldTy::Embedded(embedded) = &app_field.ty else {
        // A relation hop, or a primitive with steps left over: not an embedded
        // leaf either way.
        return None;
    };
    match schema.app.get_model(embedded.target)? {
        toasty::schema::app::Model::EmbeddedStruct(e) => {
            let MappingField::Struct(ms) = mapping_field else {
                return None;
            };
            descend(schema, &e.fields, &ms.fields, rest)
        }
        toasty::schema::app::Model::EmbeddedEnum(e) => {
            let MappingField::Enum(me) = mapping_field else {
                return None;
            };
            // A variant consumes two steps: the variant index, then the field.
            let (variant_index, tail) = rest.split_first()?;
            let variant = me.variants.get(*variant_index)?;
            let payloads: Vec<_> = e.variant_fields(*variant_index).collect();
            descend(schema, &payloads, &variant.fields, tail)
        }
        _ => None,
    }
}

/// The column a single mapping field occupies, if it is a leaf.
fn column_of(schema: &toasty_core::Schema, field: &MappingField) -> Option<LeafField> {
    let MappingField::Primitive(p) = field else {
        // A path stopping on a struct or enum names no single column, and a
        // relation stores none.
        return None;
    };
    let column = &schema.db.tables[p.column.table.0].columns[p.column.index];
    Some(LeafField {
        name: column.name.clone(),
        label: capitalize(&column.name.replace('_', " ")),
        // Every column under an embedded step is storage-nullable: only the
        // matching variant writes a value.
        nullable: true,
    })
}

/// Whether `field` is a `#[document]`: a *primitive* whose storage is a model,
/// so its inner fields collapse into the one column named after it.
fn is_document(field: &toasty::schema::app::Field) -> bool {
    matches!(
        &field.ty,
        toasty::schema::app::FieldTy::Primitive(p)
            if matches!(p.ty, toasty_core::stmt::Type::Model(_))
    )
}

/// The embedded model an app field targets, if it is embedded.
fn app_embedded<'a>(
    schema: &'a toasty_core::Schema,
    field: &toasty::schema::app::Field,
) -> Option<&'a toasty::schema::app::Model> {
    let toasty::schema::app::FieldTy::Embedded(embedded) = &field.ty else {
        return None;
    };
    schema.app.get_model(embedded.target)
}

/// The app-level field `steps` reaches, used by the variant root to find the
/// enum's payload list before descending.
///
/// Recurses through embedded structs: the variant root's parent path can walk
/// through them (an embedded struct holding the enum), not just one step.
fn app_field_at<'a>(
    schema: &'a toasty_core::Schema,
    fields: &'a [toasty::schema::app::Field],
    steps: &[usize],
) -> Option<&'a toasty::schema::app::Field> {
    let (first, rest) = steps.split_first()?;
    let field = fields.get(*first)?;
    if rest.is_empty() {
        return Some(field);
    }
    match app_embedded(schema, field)? {
        toasty::schema::app::Model::EmbeddedStruct(e) => app_field_at(schema, &e.fields, rest),
        // An enum nested in the parent path consumes two steps per level: the
        // variant index, then a variant-local field index. `.nth` walks the
        // enum's global list in variant order, which is what the generated
        // accessor's index means.
        toasty::schema::app::Model::EmbeddedEnum(e) => {
            let (variant, tail) = rest.split_first()?;
            let field = e.variant_fields(*variant).nth(*tail.first()?)?;
            if tail.len() == 1 {
                Some(field)
            } else {
                app_field_at(schema, std::slice::from_ref(field), &tail[1..])
            }
        }
        toasty::schema::app::Model::Root(_) => None,
    }
}

/// The mapping field `steps` reaches, used by the variant root to find the
/// enum's per-variant mappings before descending.
fn mapping_field_at<'a>(fields: &'a [MappingField], steps: &[usize]) -> Option<&'a MappingField> {
    let (first, rest) = steps.split_first()?;
    let field = fields.get(*first)?;
    if rest.is_empty() {
        return Some(field);
    }
    match field {
        MappingField::Struct(s) => mapping_field_at(&s.fields, rest),
        MappingField::Enum(e) => {
            let (variant, tail) = rest.split_first()?;
            mapping_field_at(&e.variants.get(*variant)?.fields, tail)
        }
        _ => None,
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

/// Whether `field` is backed by a unique index it participates in.
///
/// Uniqueness is not a property of a `Field`: Toasty stores it on the model's
/// index list, so this needs the owning `ModelRoot` too. The primary key is
/// excluded — a key column is unique by construction, not by a declared
/// constraint.
///
/// A **composite** unique index counts too (GH #88). `#[unique(tenant_id,
/// email)]` is how a tenant-scoped resource expresses "unique within the
/// tenant", and the app-side pre-check has to recognise it or the field's
/// `unique()` declaration would be silently dead. Recognizing the index is not
/// the same as checking it exactly: the pre-check probes the field's value
/// inside `R::query`'s scope, so it enforces the constraint only when that
/// scope matches the index's remaining components — which is the arrangement
/// `#[unique(tenant_id, ..)]` on a `requires_tenant` resource produces, and
/// which `the_flattened_name_participates_in_allow_list_and_validation`-style
/// showcase coverage pins. Declaring `unique()` on a field whose index carries
/// components outside the resource's scope stays a gap (upstream #117 is the
/// real fix: a driver predicate would let the write itself report the field).
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
        index.unique && !index.primary_key && index.fields.iter().any(|f| f.field == field.id)
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
