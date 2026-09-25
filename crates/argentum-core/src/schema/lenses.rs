//! Field lenses — typed Toasty paths and their app-level metadata.
//!
//! Base bridge layer (with `pk`): these two modules are the only schema modules
//! that name `toasty_core` (upstream #114/#183), so upstream churn has one
//! blast radius.
//!
//! [`lens_field`] hands callers the built `app::Field`, so name, label,
//! nullability, storage name, `FieldTy`, `auto` and `constraints` all come from
//! one walk; [`lens_field_unique`] covers uniqueness, which `Field` does not
//! carry because Toasty keeps it on the model's index list.
//!
//! [`FieldResolver`] is the schema-aware walk: with the app schema in hand an
//! embedded path resolves to its **flattened storage column**, so
//! `TextInput::r#for_post(...)`-style bindings reach a field inside an embedded
//! struct or a `#[document]`. Without a schema the single-segment rule applies,
//! because the owned `app::Model` cannot see embedded models.

use toasty_core::stmt::PathRoot;
use topcoat::context::Cx;

/// Spec alias — ADR-0001 typed lens. Currently uses `toasty::stmt::Path` directly;
/// a richer `FieldLens` trait will replace this alias if Toasty exposes the
/// metadata walk directly (see, upstream issue #183).
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
    /// Whether the leaf is nullable. Every leaf the walk resolves reports
    /// `true`: an embedded leaf is never required by default, because only the
    /// matching enum variant writes a variant payload's column. That is this
    /// walk's policy rather than the compiled column's own nullability — the
    /// flattened column of a required embedded struct is `NOT NULL` — so it is
    /// the *binding* default, not a storage fact.
    pub(crate) nullable: bool,
}

/// The compiled schema for this request, or `None` when no `Db` is in context.
///
/// `Db::schema()` is public and gives all three halves the walk needs: `.app`
/// carries the embedded models the owned `Model::schema()` cannot see,
/// `.mapping` records which physical column each field resolves to, and `.db`
/// holds the physical table and column names. So the request path can bind an
/// embedded lens without opening a connection.
///
/// Borrowed, never cloned, and optional: a bare `CxTestBuilder` has no `Db`, and
/// then the single-segment rule applies, so a schema-less test fails
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

    /// Whether this request carries an app schema at all (a `Db` in context).
    ///
    /// The leaf constructors fall back to the single-segment rule without one;
    /// a *value* binding has no fallback — every key comes from the schema — so
    /// its entry points say so rather than reporting a traversal-lens error.
    pub(crate) fn has_schema(&self) -> bool {
        self.schema.is_some()
    }

    /// Resolve a lens to its leaf field.
    ///
    /// With a schema this walks the whole projection, so an embedded step lands
    /// on the flattened column. Without one it falls back to the single-segment
    /// rule and panics on a traversal lens — loudly, because silently binding
    /// the first segment misbinds in release.
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
        // `require_single_segment` reject a traversal lens.
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
    /// - the **app schema** drives the traversal, because it is the only side that knows a field is
    ///   a `#[document]` — a *primitive* whose storage is a model, so its inner fields collapse
    ///   into the one column named after it;
    /// - the **compiled mapping** names the column. `Db::schema().mapping` records, per model,
    ///   which column every field resolves to — flattened embedded structs, enum discriminant and
    ///   payload columns, shared columns, and a document's single column alike — so this reads the
    ///   name off `db::Table` rather than re-deriving Toasty's naming rules.
    ///
    /// Two roots reach here: a **model** root for a plain path (`seo.title`), and
    /// a **variant** root for an enum payload accessor
    /// (`media.video().poster().url()`), where the generated accessor rebases
    /// onto the variant and the projection's steps are variant-local.
    ///
    /// Only embedded steps are followed. A relation hop yields `None`: this
    /// exists for embedded binding, and binding anything else would reintroduce
    /// the misbind the single-segment rule guards against.
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
                let payloads: Vec<_> = e.variant_fields(variant_id.index).iter().collect();
                descend(
                    schema,
                    &payloads,
                    &variant.fields,
                    path.projection.as_slice(),
                )
            }
        }
    }

    /// Enumerate the bindable surface of the embedded **value** at `path`.
    /// [`Self::resolve`] answers "which column does this one leaf occupy"; this
    /// answers "what does this whole value consist of" — every leaf column,
    /// plus the discriminant for an enum — which is what a value codec and a
    /// generated form need.
    ///
    /// `path` addresses the embedded field itself (`Post::fields().seo()`,
    /// `Post::fields().publication()`), not one of its leaves: a variant-rooted
    /// path names a *variant* of a value, not the value, and yields `None`.
    pub(crate) fn resolve_embedded_value<M, T>(
        &self,
        path: FieldLens<M, T>,
    ) -> Option<EmbeddedValueSpec>
    where
        M: toasty::schema::Model,
    {
        let schema = self.schema?;
        let core_path: toasty_core::stmt::Path = path.into();
        let PathRoot::Model(id) = core_path.root else {
            return None;
        };
        let owner = M::schema();
        let root = schema.app.get_model(id)?.as_root()?;
        // The same identity check `walk_embedded` makes: an accessor's
        // `ModelId` and the schema's can come from different `models!(..)`
        // expansions, so verify the root by name before trusting any index.
        if root.name.upper_camel_case() != owner.as_root()?.name.upper_camel_case() {
            return None;
        }
        let mapping = schema.mapping.models.get(&root.id)?;
        let steps = core_path.projection.as_slice();
        let app_field = app_field_at(schema, &root.fields, steps)?;
        let mapping_field = mapping_field_at(&mapping.fields, steps)?;
        let mut columns = Vec::new();
        let enum_spec = match (app_embedded(schema, app_field)?, mapping_field) {
            (toasty::schema::app::Model::EmbeddedStruct(e), MappingField::Struct(ms)) => {
                collect_columns(schema, &e.fields, &ms.fields, &mut columns)?;
                None
            }
            (toasty::schema::app::Model::EmbeddedEnum(e), MappingField::Enum(me)) => {
                collect_enum_columns(schema, e, me, &mut columns)?;
                let discriminant =
                    column_name(schema, &MappingField::Primitive(me.discriminant.clone()))?;
                let variants = e
                    .variants
                    .iter()
                    .map(|v| {
                        discriminant_text(&v.discriminant)
                            .map(|value| (value, capitalize(&v.name.snake_case())))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(crate::schema::embedded::EnumSpec::new(
                    discriminant,
                    variants,
                ))
            }
            _ => return None,
        };
        Some(EmbeddedValueSpec { columns, enum_spec })
    }
}

/// The column one mapping field occupies, by name.
fn column_name(schema: &toasty_core::Schema, field: &MappingField) -> Option<String> {
    column_of(schema, field).map(|leaf| leaf.name)
}

/// Collect every leaf column under one embedded level.
///
/// A relation inside an embedded value is not bindable and yields `None`: this
/// walk exists for value binding, the same boundary GH #100 draws for
/// single lenses.
fn collect_columns(
    schema: &toasty_core::Schema,
    app_fields: &[toasty::schema::app::Field],
    mapping_fields: &[MappingField],
    out: &mut Vec<String>,
) -> Option<()> {
    for (app_field, mapping_field) in app_fields.iter().zip(mapping_fields) {
        if is_document(app_field) {
            // A `#[document]`'s inner fields share its one column, so a *value*
            // codec has nothing per-field to bind: refuse rather than hand back
            // one column for several fields (the misbind GH #100 guards
            // against). Leaf binding still reaches a document's column.
            return None;
        }
        match &app_field.ty {
            toasty::schema::app::FieldTy::Primitive(_) => {
                push_column(schema, mapping_field, out)?;
            }
            toasty::schema::app::FieldTy::Embedded(embedded) => {
                match schema.app.get_model(embedded.target)? {
                    toasty::schema::app::Model::EmbeddedStruct(e) => {
                        let MappingField::Struct(ms) = mapping_field else {
                            return None;
                        };
                        collect_columns(schema, &e.fields, &ms.fields, out)?;
                    }
                    toasty::schema::app::Model::EmbeddedEnum(e) => {
                        let MappingField::Enum(me) = mapping_field else {
                            return None;
                        };
                        collect_enum_columns(schema, e, me, out)?;
                    }
                    toasty::schema::app::Model::Root(_) => return None,
                }
            }
            _ => return None,
        }
    }
    Some(())
}

/// Collect every variant's payload columns under one embedded enum.
fn collect_enum_columns(
    schema: &toasty_core::Schema,
    e: &toasty::schema::app::EmbeddedEnum,
    me: &toasty_core::schema::mapping::FieldEnum,
    out: &mut Vec<String>,
) -> Option<()> {
    // The discriminant column first: it is a form key of this value (the
    // control that carries the variant), whether the enum is the value itself
    // or nested inside one.
    push_column(
        schema,
        &MappingField::Primitive(me.discriminant.clone()),
        out,
    )?;
    for (index, variant) in me.variants.iter().enumerate() {
        collect_columns(schema, e.variant_fields(index), &variant.fields, out)?;
    }
    Some(())
}

/// Push a column name once: a `#[shared(..)]` column is declared by several
/// variants and is still one column.
fn push_column(
    schema: &toasty_core::Schema,
    field: &MappingField,
    out: &mut Vec<String>,
) -> Option<()> {
    let name = column_name(schema, field)?;
    if !out.contains(&name) {
        out.push(name);
    }
    Some(())
}

/// The text an embedded enum's discriminant stores.
///
/// Toasty's discriminants are integers or strings, and the form carries that
/// same text — so what a submission posts is what a row stores. Anything else
/// is reported rather than guessed.
fn discriminant_text(value: &toasty_core::stmt::Value) -> Option<String> {
    use toasty_core::stmt::Value;
    match value {
        Value::Bool(v) => Some(v.to_string()),
        Value::I8(v) => Some(v.to_string()),
        Value::I16(v) => Some(v.to_string()),
        Value::I32(v) => Some(v.to_string()),
        Value::I64(v) => Some(v.to_string()),
        Value::U8(v) => Some(v.to_string()),
        Value::U16(v) => Some(v.to_string()),
        Value::U32(v) => Some(v.to_string()),
        Value::U64(v) => Some(v.to_string()),
        Value::String(v) => Some(v.clone()),
        _ => None,
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
            let payloads: Vec<_> = e.variant_fields(*variant_index).iter().collect();
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
        // Reported nullable by policy, not read off the column: an embedded
        // leaf is never required by default, because only the matching enum
        // variant writes a variant payload's column. The compiled column can be
        // `NOT NULL` — an embedded struct's flattened column is.
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

/// The bindable surface of one embedded **value**: every column its
/// fields occupy, plus — for an enum — the discriminant column and each
/// variant's stored value.
///
/// [`FieldResolver::resolve`] answers "which column does this one leaf occupy";
/// this answers "what does this whole value consist of", which is what a value
/// codec and a generated form need. Both read the same two authorities: the app
/// schema decides the structure, the compiled mapping names every column.
#[derive(Debug, Clone)]
pub(crate) struct EmbeddedValueSpec {
    /// Every form key the value occupies, in declaration order and
    /// deduplicated: each leaf column, and the discriminant column of every
    /// enum the value contains — the top-level one and any nested inside a
    /// struct or a variant. A `#[shared(..)]` column declared by several
    /// variants appears once, because it *is* one column.
    pub(crate) columns: Vec<String>,
    /// `Some` for an embedded enum: its discriminant column and its variants
    /// (each one's stored value and the name it is labelled with). The one
    /// `EnumSpec` type is shared with the public seam.
    pub(crate) enum_spec: Option<crate::schema::embedded::EnumSpec>,
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
        // variant index, then a variant-local field index. `variant_fields`
        // returns the variant's own slice of the enum's global field list
        // (upstream 7ff180db), so the second step indexes it directly — the
        // generated accessor's index is variant-local, which is the same field
        // either way.
        //
        // The variant step is bounds-checked here first: `variant_fields`
        // indexes `variants[i]` and panics out of range, and this walk answers
        // `None` for a path it cannot resolve (a wrong lens must not abort a
        // request).
        toasty::schema::app::Model::EmbeddedEnum(e) => {
            let (variant, tail) = rest.split_first()?;
            e.variants.get(*variant)?;
            let field = e.variant_fields(*variant).get(*tail.first()?)?;
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
/// Traversal lenses are rejected: a multi-step path has no single
/// field name, and silently binding its first segment misbinds in release.
/// Use [`FieldResolver`] to bind an embedded path instead.
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
/// A **composite** unique index counts too: `#[unique(tenant_id, email)]` is how
/// a tenant-scoped resource expresses "unique within the tenant", and the
/// app-side pre-check has to recognise it or the field's `unique()` declaration
/// is silently dead. Recognizing the index is not checking it exactly: the
/// pre-check probes inside the tenant-scoped query's scope, so it enforces the
/// constraint only when that scope matches the index's remaining components.
/// An index with components outside the scope stays a gap, and it is not
/// checkable here because a query's filters are not introspectable; see
/// `Resource::query`'s note and upstream #117.
///
/// This reports declared schema uniqueness, not a global guarantee: SQL permits
/// multiple `NULL`s in a unique index, and enum-variant columns are
/// storage-nullable, so a nullable unique field can still repeat.
pub(crate) fn lens_field_unique(
    field: &toasty::schema::app::Field,
    model: &toasty::schema::app::ModelRoot,
) -> bool {
    model.indices.iter().any(|index| {
        index.unique && !index.primary_key && index.fields.iter().any(|f| f.field == field.id)
    })
}

/// Panic unless a lens path addresses exactly one field.
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

/// A field's human label from its storage name.
///
/// Sentence case, with the underscores a Rust column name carries read as
/// spaces: `word_count` is "Word count", not "Word_count". A label is the one
/// place a column name becomes prose, so it should not leak the identifier —
/// which is what `Media_poster_url` and `Seo_title` did everywhere an embedded
/// or document leaf rendered its own name (#192). An explicit
/// [`.label(..)`](crate::schema::TextInput::label) still wins.
pub(crate) fn capitalize(s: &str) -> String {
    let spaced = s.replace('_', " ");
    let mut c = spaced.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {

    use toasty::schema::Model;
    use topcoat::context::CxTestBuilder;

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

    // === the resolver walk ==================================================
    //
    // `FieldResolver` reads the compiled schema the request carries, so these
    // tests build a `Db` over one model carrying every shape the walk branches
    // on, and drive the walk through its two production entries.

    /// An embedded struct: its leaves flatten into the parent's table.
    #[derive(Debug, Clone, toasty::Embed)]
    struct Seo {
        title: String,
        description: String,
    }

    /// The deepest level of `media_poster_credit_author`.
    #[derive(Debug, Clone, toasty::Embed)]
    struct Credit {
        author: String,
    }

    #[derive(Debug, Clone, toasty::Embed)]
    struct Poster {
        url: String,
        credit: Credit,
    }

    /// An embedded enum with a struct nested inside a variant: the path
    /// `media().video().poster().credit().author()` is three levels deep and
    /// still lands on one flat column.
    #[derive(Debug, Clone, toasty::Embed)]
    enum Media {
        #[column(variant = 1)]
        Image { url: String, alt: String },
        #[column(variant = 2)]
        Video { video_url: String, poster: Poster },
    }

    /// Two variants declaring one `#[shared(timestamp)]` column.
    #[derive(Debug, Clone, toasty::Embed)]
    enum Publication {
        #[column(variant = 1)]
        Scheduled {
            #[shared(timestamp)]
            scheduled_at: String,
            scheduled_for: String,
        },
        #[column(variant = 2)]
        Published {
            #[shared(timestamp)]
            published_at: String,
            canonical_url: String,
        },
    }

    /// A struct holding an enum: the nested discriminant is a key of the value
    /// too, not only its payloads.
    #[derive(Debug, Clone, toasty::Embed)]
    struct Wrapper {
        label: String,
        inner: Media,
    }

    /// A `#[document]`: a primitive whose storage is a model, so its inner
    /// fields share one column named after the field.
    #[derive(Debug, Clone, toasty::Embed)]
    struct Stats {
        word_count: i64,
        read_minutes: i64,
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct LensPost {
        #[key]
        #[auto]
        id: uuid::Uuid,
        title: String,
        seo: Seo,
        media: Media,
        publication: Publication,
        wrapper: Wrapper,
        #[document]
        stats: Stats,
    }

    /// A second root model for the identity guard. Its fields are ordered so
    /// that a lookup trusting the *id* would land on a different column than
    /// the lens names: `LensPost.seo` is index 2, `Impostor.wrapper` is index 2.
    #[derive(Debug, Clone, toasty::Model)]
    struct Impostor {
        #[key]
        #[auto]
        id: uuid::Uuid,
        seo: Seo,
        wrapper: Wrapper,
    }

    /// The request context a resolver reads its schema from.
    async fn cx_with(models: toasty::schema::ModelSet) -> Cx {
        let db = toasty::Db::builder()
            .models(models)
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        CxTestBuilder::new().app_context(db).build()
    }

    /// The context `LensPost`'s lenses resolve against.
    async fn lens_cx() -> Cx {
        cx_with(toasty::models!(LensPost)).await
    }

    /// A model set whose root is `Impostor` wearing `LensPost`'s id.
    ///
    /// Two Rust model types never share a `ModelId` in one process, so the one
    /// way an id can name a foreign model is an assembled set — which is what
    /// the guard exists for: it trusts the root's **name**, never the id.
    fn models_with_a_foreign_root() -> toasty::schema::ModelSet {
        let mut set = toasty::schema::ModelSet::new();
        let mut model = Impostor::schema();
        let toasty_core::schema::app::Model::Root(root) = &mut model else {
            panic!("a #[derive(Model)] type builds a root model");
        };
        let id = <LensPost as toasty::schema::Model>::id();
        root.id = id;
        // A field's own `FieldId` names its model too, so the forgery has to
        // carry through or the root would be inconsistent with its fields.
        for field in &mut root.fields {
            field.id.model = id;
        }
        set.add(model);
        // The forged root embeds both, so their models must be in the set too.
        <Seo as toasty::schema::Field>::register(&mut set);
        <Wrapper as toasty::schema::Field>::register(&mut set);
        set
    }

    /// A plain leaf: a single segment on a model root never enters the walk, so
    /// the owned app field answers name, label and nullability.
    #[tokio::test]
    async fn a_plain_leaf_resolves_through_the_app_field() {
        let cx = lens_cx().await;
        let leaf = FieldResolver::from_cx(&cx).resolve(LensPost::fields().title());
        assert_eq!(leaf.name, "title");
        assert_eq!(leaf.label, "Title");
        assert!(
            !leaf.nullable,
            "a required top-level column is not storage-nullable"
        );
    }

    /// One embedded step: the walk follows it and names the flattened column.
    #[tokio::test]
    async fn a_leaf_in_an_embedded_struct_resolves_to_its_flattened_column() {
        let cx = lens_cx().await;
        let leaf = FieldResolver::from_cx(&cx).resolve(LensPost::fields().seo().title());
        assert_eq!(leaf.name, "seo_title");
        assert_eq!(
            leaf.label, "Seo title",
            "a label reads the storage name as prose"
        );
        assert!(
            leaf.nullable,
            "an embedded leaf is never required by default"
        );
    }

    /// A variant-rooted path through nested structs, three levels deep, landing
    /// on one flat column.
    #[tokio::test]
    async fn a_leaf_in_a_struct_nested_in_an_enum_variant_resolves_to_one_column() {
        let cx = lens_cx().await;
        let resolver = FieldResolver::from_cx(&cx);

        // media.image().url() — the other variant, one level down.
        assert_eq!(
            resolver
                .resolve(LensPost::fields().media().image().url())
                .name,
            "media_url"
        );
        // media.video().poster().url() — a struct one level inside the variant.
        assert_eq!(
            resolver
                .resolve(LensPost::fields().media().video().poster().url())
                .name,
            "media_poster_url"
        );
        // media.video().poster().credit().author() — three levels, one column.
        let leaf = resolver.resolve(
            LensPost::fields()
                .media()
                .video()
                .poster()
                .credit()
                .author(),
        );
        assert_eq!(leaf.name, "media_poster_credit_author");
        assert_eq!(leaf.label, "Media poster credit author");
        assert!(leaf.nullable);
    }

    /// A variant-rooted path whose *parent* walks through an embedded struct:
    /// the payload accessor rebases onto the variant, so the parent path has
    /// two steps and both halves of the walk have to follow them to reach the
    /// enum — the app side to find its payload list, the mapping side to find
    /// its per-variant columns.
    #[tokio::test]
    async fn a_variant_rooted_path_through_an_embedded_struct_resolves() {
        let cx = lens_cx().await;
        let resolver = FieldResolver::from_cx(&cx);
        assert_eq!(
            resolver
                .resolve(LensPost::fields().wrapper().inner().image().url())
                .name,
            "wrapper_inner_url"
        );
        assert_eq!(
            resolver
                .resolve(LensPost::fields().wrapper().inner().video().video_url())
                .name,
            "wrapper_inner_video_url"
        );
    }

    /// `#[shared(timestamp)]`: both variants' leaves name the one column the
    /// identifier declares, and a non-shared payload keeps its own.
    #[tokio::test]
    async fn a_shared_column_resolves_for_every_variant_that_declares_it() {
        let cx = lens_cx().await;
        let resolver = FieldResolver::from_cx(&cx);
        assert_eq!(
            resolver
                .resolve(LensPost::fields().publication().scheduled().scheduled_at())
                .name,
            "publication_timestamp"
        );
        assert_eq!(
            resolver
                .resolve(LensPost::fields().publication().published().published_at())
                .name,
            "publication_timestamp"
        );
        assert_eq!(
            resolver
                .resolve(LensPost::fields().publication().published().canonical_url())
                .name,
            "publication_canonical_url"
        );
    }

    /// A `#[document]`: its inner fields share its one column, so the walk
    /// stops at the document however many steps remain.
    #[tokio::test]
    async fn a_document_leaf_resolves_to_the_document_column() {
        let cx = lens_cx().await;
        let leaf = FieldResolver::from_cx(&cx).resolve(LensPost::fields().stats().word_count());
        assert_eq!(leaf.name, "stats");
        assert_eq!(leaf.label, "Stats");
        assert!(
            leaf.nullable,
            "a document leaf is never required by default either"
        );
    }

    /// The identity guard, on the id lookup: a schema that does not carry the
    /// lens's root model at all resolves to nothing.
    #[tokio::test]
    async fn a_root_model_the_schema_does_not_carry_resolves_to_nothing() {
        let cx = cx_with(toasty::models!(Impostor)).await;
        let resolver = FieldResolver::from_cx(&cx);
        assert!(resolver.has_schema(), "the Db carries the app schema");
        assert!(
            request_schema(&cx)
                .expect("the Db carries the app schema")
                .app
                .get_model(<LensPost as Model>::id())
                .is_none(),
            "this schema carries no model under the lens's id"
        );
        assert!(
            resolver
                .resolve_embedded_value(LensPost::fields().seo().into())
                .is_none(),
            "a value cannot resolve against a schema that has no LensPost"
        );
    }

    /// ... and through `resolve` the same path panics instead of quietly
    /// resolving to nothing (the policy).
    #[tokio::test]
    #[should_panic(expected = "does not resolve to a single column")]
    async fn a_root_model_the_schema_does_not_carry_refuses_a_leaf_lens() {
        let cx = cx_with(toasty::models!(Impostor)).await;
        let _ = FieldResolver::from_cx(&cx).resolve(LensPost::fields().seo().title());
    }

    /// The identity guard, on the name: an id that *is* in the schema but names
    /// another model resolves to nothing, even though the impostor's field at
    /// that index would have answered with a column (`wrapper_label`).
    #[tokio::test]
    async fn an_id_that_names_another_model_resolves_to_nothing() {
        let cx = cx_with(models_with_a_foreign_root()).await;
        let resolver = FieldResolver::from_cx(&cx);
        let schema = request_schema(&cx).expect("the Db carries the app schema");
        let root = schema
            .app
            .get_model(<LensPost as Model>::id())
            .expect("the forged root answers the lens's id")
            .as_root()
            .expect("a root model");
        assert_eq!(
            root.name.upper_camel_case(),
            "Impostor",
            "the id is the lens's, the name is another model's"
        );
        // What an id-trusting walk would bind: the forged root's field at the
        // lens's first step is `wrapper`, whose own leaf is another column.
        let mapping = schema
            .mapping
            .models
            .get(&root.id)
            .expect("the forged root is mapped");
        assert_eq!(
            descend(schema, &root.fields, &mapping.fields, &[2, 0])
                .expect("the forged root's field 2 has a leaf")
                .name,
            "wrapper_label",
            "only the name check keeps this column from being bound for `seo.title`"
        );
        assert!(
            resolver
                .resolve_embedded_value(LensPost::fields().seo().into())
                .is_none(),
            "the root at this id is Impostor, so no index may be trusted"
        );
    }

    /// ... and through `resolve` it panics rather than misbinding.
    #[tokio::test]
    #[should_panic(expected = "does not resolve to a single column")]
    async fn an_id_that_names_another_model_refuses_a_leaf_lens() {
        let cx = cx_with(models_with_a_foreign_root()).await;
        let _ = FieldResolver::from_cx(&cx).resolve(LensPost::fields().seo().title());
    }

    /// A struct value: its leaf columns in declaration order, and no
    /// discriminant.
    #[tokio::test]
    async fn an_embedded_struct_value_lists_its_leaf_columns() {
        let cx = lens_cx().await;
        let spec = FieldResolver::from_cx(&cx)
            .resolve_embedded_value(LensPost::fields().seo().into())
            .expect("an embedded struct is a value");
        assert_eq!(spec.columns, ["seo_title", "seo_description"]);
        assert!(
            spec.enum_spec.is_none(),
            "a struct has no variant to choose"
        );
    }

    /// An enum value: the discriminant column first, then each variant's
    /// payload — the `#[shared]` column once, because it is one column.
    #[tokio::test]
    async fn an_embedded_enum_value_lists_its_discriminant_and_every_variant() {
        let cx = lens_cx().await;
        let spec = FieldResolver::from_cx(&cx)
            .resolve_embedded_value(LensPost::fields().publication().into())
            .expect("an embedded enum is a value");
        assert_eq!(
            spec.columns,
            [
                "publication",
                "publication_timestamp",
                "publication_scheduled_for",
                "publication_canonical_url",
            ],
            "the shared column appears once, and the discriminant is a key too"
        );
        let enum_spec = spec.enum_spec.expect("an enum has a variant to choose");
        assert_eq!(enum_spec.discriminant(), "publication");
        assert_eq!(enum_spec.len(), 2);
        assert_eq!(enum_spec.value_of_index(0), Some("1"));
        assert_eq!(enum_spec.name_of_index(1), Some("Published"));
    }

    /// A struct holding an enum: the nested enum's discriminant is a key of the
    /// value too, while the value itself stays a struct.
    #[tokio::test]
    async fn a_struct_value_lists_the_discriminant_of_an_enum_it_holds() {
        let cx = lens_cx().await;
        let spec = FieldResolver::from_cx(&cx)
            .resolve_embedded_value(LensPost::fields().wrapper().into())
            .expect("an embedded struct is a value");
        assert_eq!(
            spec.columns,
            [
                "wrapper_label",
                "wrapper_inner",
                "wrapper_inner_url",
                "wrapper_inner_alt",
                "wrapper_inner_video_url",
                "wrapper_inner_poster_url",
                "wrapper_inner_poster_credit_author",
            ]
        );
        assert!(
            spec.enum_spec.is_none(),
            "the value at the path is the struct, not the enum it holds"
        );
    }

    /// A variant-rooted path names one variant, not the value: value binding
    /// starts at the embedded field itself.
    #[tokio::test]
    async fn a_variant_rooted_path_is_not_an_embedded_value() {
        let cx = lens_cx().await;
        assert!(
            FieldResolver::from_cx(&cx)
                .resolve_embedded_value(LensPost::fields().media().video().video_url())
                .is_none()
        );
    }

    /// A plain column and a `#[document]` are leaves, not values: neither has a
    /// per-field surface a codec could bind.
    #[tokio::test]
    async fn a_leaf_is_not_an_embedded_value() {
        let cx = lens_cx().await;
        let resolver = FieldResolver::from_cx(&cx);
        assert!(
            resolver
                .resolve_embedded_value(LensPost::fields().title())
                .is_none(),
            "a primitive field is a leaf"
        );
        assert!(
            resolver
                .resolve_embedded_value(LensPost::fields().stats().into())
                .is_none(),
            "a #[document] stores as one column, so it has no per-field value"
        );
    }

    /// Without a `Db` there is no schema, and a value binding has no fallback.
    #[test]
    fn without_a_schema_there_is_no_walk() {
        let cx = CxTestBuilder::new().build();
        let resolver = FieldResolver::from_cx(&cx);
        assert!(!resolver.has_schema(), "a bare Cx carries no Db");
        assert!(
            resolver
                .resolve_embedded_value(LensPost::fields().seo().into())
                .is_none(),
            "a value binding needs the app schema"
        );
    }
}
