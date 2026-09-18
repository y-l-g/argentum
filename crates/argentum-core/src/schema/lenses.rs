//! Field lenses — typed Toasty paths and their app-level metadata.
//!
//! Base bridge layer (with `pk`): these two modules are the only schema
//! modules that name `toasty_core` (upstream #114/#115), so upstream
//! churn has one blast radius. Everything above reads lenses through
//! `FieldLens` and the helpers here.

/// Spec alias — ADR-0001 typed lens. Currently uses `toasty::stmt::Path` directly;
/// a richer `FieldLens` trait (carrying `FieldTy`, nullability, etc.) will replace
/// this alias when Toasty exposes the helpers publicly (see GH #11, upstream
/// issue #115).
pub type FieldLens<M, T> = toasty::stmt::Path<M, T>;

/// Resolve a typed lens to its app-level field name and capitalized label.
///
/// Hides the `Path → toasty_core::stmt::Path → projection → M::schema()` walk
/// (upstream issue #114). Used by `TextColumn` and the other table-side
/// helpers; the form inputs use [`lens_field_name_label_and_nullable`], which
/// adds the nullability their required-defaults read (GH #100, GH #147).
///
/// Traversal lenses are rejected (GH #100): a multi-step path has no single
/// field name, and silently binding its first segment misbinds in release.
pub(crate) fn lens_field_name_and_label<M, T>(path: FieldLens<M, T>) -> (String, String)
where
    M: toasty::schema::Model,
{
    let (field_name, label_str, _nullable) = lens_field_name_label_and_nullable(path);
    (field_name, label_str)
}

/// Field name, label, and nullability behind a lens in one walk (GH #100) —
/// the required-default needs all three, in `TextInput`, `Select`, and
/// `FileUpload` (GH #147); walking once keeps the single `toasty_core`
/// import site obvious.
pub(crate) fn lens_field_name_label_and_nullable<M, T>(
    path: FieldLens<M, T>,
) -> (String, String, bool)
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
    let model = M::schema();
    let (field_name, nullable) = model
        .fields()
        .get(idx)
        .map(|f| (f.name.app_unwrap().to_string(), f.nullable))
        .unwrap_or_else(|| {
            panic!(
                "field index {idx} out of bounds for {}",
                std::any::type_name::<M>()
            )
        });
    (field_name.clone(), capitalize(&field_name), nullable)
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
}
