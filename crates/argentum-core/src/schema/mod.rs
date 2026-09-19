//! Unified Schema primitive — layout blocks that compose via `view!`.
//!
//! `Schema` is a container for `Section`, `Group`, `Grid` and `Text` nodes.
//! Each node renders through Topcoat's `view!` macro; `Schema::render`
//! combines them. The API mirrors Filament's `Schema::new(( ... ))` tuple
//! form via the `IntoSchema` trait.
//!
//! Bridge note: `lens_field` is the one walk reaching into `toasty_core`
//! (upstream issue #114), alongside the `pk_*` bridge helpers and `cursor.rs`
//! cursor values. It hands back the built `app::Field`, so field metadata no
//! longer needs a helper per property; uniqueness comes from
//! `lens_field_unique`, since Toasty keeps it on the model's index list rather
//! than the field. Retire the walk when Toasty exposes it (upstream #183).

mod fields;
mod layouts;
mod lenses;
mod pk;
mod relationship;
mod tree;

pub use fields::{FileUpload, Select, TextInput};
// GH #173: the placeholder leaf stays reachable to the unit tests without
// widening the public surface; the render arm (tree.rs) imports it directly.
#[cfg(test)]
pub(crate) use fields::Text;
pub use layouts::{Grid, Group, Repeater, Section, Tabs, Wizard};
pub use lenses::FieldLens;
pub(crate) use lenses::{capitalize, lens_field, lens_label};
pub(crate) use pk::{pk_eq_expr, pk_in_expr, pk_is_composite};
pub use relationship::MAX_RELATIONSHIP_OPTIONS;
pub(crate) use relationship::OptionLoadError;
pub use tree::IntoSchema;
pub(crate) use tree::{Node, RenderSource, for_each_field, walk_repeater_absence};

use std::collections::{HashMap, HashSet};

use topcoat::runtime::Signal;
use topcoat::{Result, context::Cx, view::*};

/// The container that composes layout blocks.
#[derive(Debug, Default)]
pub struct Schema {
    pub(crate) nodes: Vec<Node>,
}

impl Schema {
    /// Whether this schema declares nothing to render (GH #138).
    ///
    /// `Panel::build` refuses a resource that allows create but declares no
    /// fields: the form would render empty and silently accept nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Build a `Schema` from any `IntoSchema` (single node, tuple, or `Schema`).
    ///
    /// Panics on duplicate field names (GH #100): two inputs sharing one name
    /// render two `<input name="x">`, POST one value to both, and collapse to
    /// one validation rule via last-wins `map.insert`.
    pub fn new(children: impl IntoSchema) -> Self {
        let schema = children.into_schema();
        schema.assert_unique_field_names();
        schema
    }

    /// An empty schema (no nodes).
    pub fn empty() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Render the schema to a `View` (no DB access).
    pub async fn render<'a>(&self, cx: &'a Cx) -> Result<BoxView<'a>> {
        self.render_with(cx, &HashMap::new(), &HashMap::new()).await
    }

    /// Render with pre-filled values and inline errors.
    pub async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        self.render_source(cx, &RenderSource::Static { values, errors })
            .await
    }

    /// Render with signal-bound values (GH #154 §4).
    ///
    /// Like [`Self::render_with`], but every [`TextInput`] whose field name
    /// appears in `values` renders its control against that signal:
    /// `:value`/`@input` keep the field and the signal in step, so a shard
    /// re-render reads what the user typed. `errors` render through the same
    /// slots as `render_with`. A field with no signal — and every other node
    /// kind — falls back to its static render.
    pub async fn render_live_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, Signal<String>>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        self.render_source(cx, &RenderSource::Live { values, errors })
            .await
    }

    /// The one node walk: static values render as before, live values bind
    /// the fields the caller supplied signals for.
    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        let mut views = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            views.push(Box::pin(node.render_source(cx, source)).await?.boxed());
        }
        Ok(view! {
            cx =>
            for v in views {
                (v)
            }
        }
        .boxed())
    }

    /// Collect field names for validation (TextInput + Select).
    pub fn field_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for node in &self.nodes {
            for_each_field(node, &mut |n| match n {
                Node::TextInput(f) => out.push(f.field_name().to_string()),
                Node::Select(f) => out.push(f.field_name().to_string()),
                Node::FileUpload(f) => out.push(f.field_name().to_string()),
                _ => {}
            });
        }
        out
    }

    /// Keys in `values` that no declared input owns, sorted (GH #89).
    ///
    /// Framework-level allow-list seam, enforced by the create/edit POST
    /// handlers (unknown keys → 400): record handlers already whitelist via
    /// per-field `.get(..)`, but a generic impl iterating `values` would
    /// silently promote `role`/`tenant_id`/handler keys (`csrf_token`,
    /// `confirm`, `ids`) to client-controlled writes. The transport keys the
    /// handlers own (`csrf_token`, `clear_<field>`) are stripped before the
    /// record fns run (GH #148), so a generic impl cannot promote those
    /// either; `confirm`/`ids` are only read, never written. Callers should
    /// reject or ignore the rest (at least `debug_assert!` in tests); handler
    /// keys must be filtered by the caller before calling this.
    pub fn unknown_keys(&self, values: &HashMap<String, String>) -> Vec<String> {
        use std::collections::HashSet;
        let known: HashSet<String> = self.field_names().into_iter().collect();
        let mut out: Vec<String> = values
            .keys()
            .filter(|k| !known.contains(k.as_str()))
            .cloned()
            .collect();
        out.sort();
        out
    }

    fn assert_unique_field_names(&self) {
        let names = self.field_names();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            assert!(
                seen.insert(name.clone()),
                "duplicate field name '{name}': each Schema input needs a distinct field (GH #100)"
            );
        }
    }

    /// Build a map of `field_name -> TextInput` for validation.
    pub fn text_inputs(&self) -> HashMap<String, TextInput> {
        let mut map = HashMap::new();
        for node in &self.nodes {
            for_each_field(node, &mut |n| {
                if let Node::TextInput(f) = n {
                    map.insert(f.field_name().to_string(), (**f).clone());
                }
            });
        }
        map
    }

    /// Build a map of `field_name -> Select` for validation.
    pub fn select_inputs(&self) -> HashMap<String, Select> {
        let mut map = HashMap::new();
        for node in &self.nodes {
            for_each_field(node, &mut |n| {
                if let Node::Select(f) = n {
                    map.insert(f.field_name().to_string(), (**f).clone());
                }
            });
        }
        map
    }

    /// Build a map of `field_name -> FileUpload` for validation.
    pub fn file_uploads(&self) -> HashMap<String, FileUpload> {
        let mut map = HashMap::new();
        for node in &self.nodes {
            for_each_field(node, &mut |n| {
                if let Node::FileUpload(f) = n {
                    map.insert(f.field_name().to_string(), (**f).clone());
                }
            });
        }
        map
    }

    /// Whether this schema (including nested Section/Group/Grid/Repeater/Tabs/Wizard)
    /// contains a [`FileUpload`]. `Panel` uses it to emit
    /// `enctype="multipart/form-data"` only on forms that need it (GH #73).
    pub fn has_file_upload(&self) -> bool {
        let mut found = false;
        for node in &self.nodes {
            for_each_field(node, &mut |n| {
                if matches!(n, Node::FileUpload(_)) {
                    found = true;
                }
            });
        }
        found
    }

    /// Validate submitted values against declared inputs (GH #89).
    ///
    /// Absent keys are treated as `""` for validation; update record fns must
    /// therefore only write keys present in the submission, or an omitted
    /// optional field silently blanks the stored value. Use
    /// [`Self::unknown_keys`] to allow-list POST keys.
    pub fn validate(&self, values: &HashMap<String, String>) -> HashMap<String, Vec<String>> {
        // Classify the repeaters first (GH #147): an all-empty group is
        // "absent" — an untouched group submits empty strings (or omits the
        // keys), both treated as absent — so its inner inputs must not fail
        // the submit for any requiredness. A `required` group answers with
        // its one label-keyed error instead, and required repeaters nested
        // inside an absent group are suppressed with it. A partially filled
        // group (any inner value non-empty) enforces inner `required` as
        // usual. Whitespace-only values count as empty, matching the
        // codebase-wide trim convention.
        let mut errors: HashMap<String, Vec<String>> = HashMap::new();
        let mut skip: HashSet<String> = HashSet::new();
        walk_repeater_absence(&self.nodes, values, &mut skip, &mut errors, false);
        let inputs = self.text_inputs();
        for (name, input) in inputs {
            if skip.contains(&name) {
                continue;
            }
            let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
            let errs = input.validate(val);
            if !errs.is_empty() {
                errors.insert(name, errs);
            }
        }
        for (name, sel) in self.select_inputs() {
            if skip.contains(&name) {
                continue;
            }
            let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
            let errs = sel.validate(val);
            if !errs.is_empty() {
                errors.insert(name, errs);
            }
        }
        for (name, fu) in self.file_uploads() {
            if skip.contains(&name) {
                continue;
            }
            let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
            let errs = fu.validate(val);
            if !errs.is_empty() {
                errors.insert(name, errs);
            }
        }
        errors
    }

    /// Field names hidden inside absent Repeater groups for these values
    /// (GH #147): the same classification `validate` uses — an all-empty
    /// group is "absent" — minus the required-group errors, which validation
    /// already reported. `check_unique` consults it so an untouched group is
    /// never unique-checked while validation calls it clean.
    pub(crate) fn absent_repeater_fields(
        &self,
        values: &HashMap<String, String>,
    ) -> HashSet<String> {
        let mut skip = HashSet::new();
        let mut discarded = HashMap::new();
        walk_repeater_absence(&self.nodes, values, &mut skip, &mut discarded, false);
        skip
    }

    /// Async validation for Select relationship existence (tenancy-aware).
    pub async fn validate_async(
        &self,
        cx: &Cx,
        values: &HashMap<String, String>,
    ) -> HashMap<String, Vec<String>> {
        let mut errors = self.validate(values);
        for (name, sel) in self.select_inputs() {
            if errors.contains_key(&name) {
                continue;
            }
            if sel.relationship.is_some() || !sel.options_static.is_empty() {
                let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
                if !val.trim().is_empty() {
                    let async_errs = sel.validate_async(cx, val).await;
                    // validate_async returns required errs plus existence; we already did required, so filter.
                    let existence_errs: Vec<String> = async_errs
                        .into_iter()
                        .filter(|e| !e.contains("is required"))
                        .collect();
                    if !existence_errs.is_empty() {
                        errors.insert(name, existence_errs);
                    }
                }
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
    fn has_file_upload_detects_nested() {
        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
            title: String,
        }
        let plain = Schema::new(TextInput::r#for(DummyUser::fields().name()));
        assert!(!plain.has_file_upload());
        let direct = Schema::new(FileUpload::r#for(Doc::fields().path()));
        assert!(direct.has_file_upload());
        // Nested inside Section/Grid/Repeater counts.
        let nested = Schema::new(Section::new("S").schema(Grid::new(2).schema((
            TextInput::r#for(DummyUser::fields().name()),
            FileUpload::r#for(Doc::fields().path()),
        ))));
        assert!(nested.has_file_upload());
        let in_repeater =
            Schema::new(Repeater::new("R").schema(FileUpload::r#for(Doc::fields().path())));
        assert!(in_repeater.has_file_upload());
    }

    #[test]
    #[should_panic(expected = "duplicate field name")]
    fn schema_rejects_duplicate_field_names() {
        let _ = Schema::new((
            TextInput::r#for(DummyUser::fields().name()),
            TextInput::r#for(DummyUser::fields().name()),
        ));
    }

    #[test]
    fn unknown_keys_flags_undeclared_post_keys() {
        let schema = Schema::new(TextInput::r#for(DummyUser::fields().name()));
        let mut values = HashMap::new();
        values.insert("name".to_string(), "Ada".to_string());
        values.insert("role".to_string(), "admin".to_string());
        values.insert("confirm".to_string(), "1".to_string());
        assert_eq!(
            schema.unknown_keys(&values),
            vec!["confirm".to_string(), "role".to_string()]
        );
        values.remove("role");
        values.remove("confirm");
        assert!(schema.unknown_keys(&values).is_empty());
    }
}
