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

/// Spec alias — ADR-0001 typed lens. Currently uses `toasty::stmt::Path` directly;
/// a richer `FieldLens` trait will replace this alias if Toasty exposes the
/// metadata walk directly (see GH #11, upstream issue #183).
pub type FieldLens<M, T> = toasty::stmt::Path<M, T>;

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
