//! Unified Schema primitive — layout blocks that compose via `view!`.
//!
//! `Schema` is a container for `Section`, `Group`, `Grid`, `Tabs` and
//! `Repeater` nodes. Each node renders through Topcoat's `view!` macro;
//! `Schema::render` combines them. The API mirrors Filament's
//! `Schema::new(( ... ))` tuple form via the `IntoSchema` trait.
//!
//! Bridge note: `lens_field` is the one walk reaching into `toasty_core`
//! (upstream issue #114), alongside the `pk_*` bridge helpers and `cursor.rs`
//! cursor values. It hands back the built `app::Field`, so field metadata no
//! longer needs a helper per property; uniqueness comes from
//! `lens_field_unique`, since Toasty keeps it on the model's index list rather
//! than the field. Retire the walk when Toasty exposes it (upstream #183).

mod embedded;
mod fields;
mod layouts;
mod lenses;
mod pk;
mod relationship;
mod tree;
mod validation;

use std::collections::{HashMap, HashSet};

pub use embedded::{
    EmbeddedForm, EnumSpec, discriminant_select, enum_spec, leaf_key, parse_leaf, read_embedded,
    submitted, write_embedded,
};
pub use fields::{FileUpload, Select, TextInput, Textarea};
pub use layouts::{Grid, Group, Repeater, Section, Tabs};
pub use lenses::FieldLens;
pub(crate) use lenses::{capitalize, lens_field, lens_field_unique, lens_label};
pub(crate) use pk::{pk_eq_expr, pk_in_expr, pk_is_composite};
pub(crate) use relationship::OptionLoadError;
pub use relationship::{MAX_RELATIONSHIP_OPTIONS, OptionSource};
use topcoat::{Result, context::Cx, view::*};
pub use tree::IntoSchema;
pub(crate) use tree::{
    Mode, Node, RenderSource, for_each_field, validate_leaf, walk_absent_groups,
};
pub use validation::TypedValue;

/// The container that composes layout blocks.
#[derive(Debug, Default)]
pub struct Schema {
    pub(crate) nodes: Vec<Node>,
}

impl Schema {
    /// Whether this schema declares nothing to render (GH #138).
    ///
    /// `Panel::build` refuses a resource that allows create but declares no
    /// fields: the form would render empty and silently accept nothing. Public
    /// since GH #187, where `Resource::view`'s default is the empty schema and
    /// [`Resource::viewed`](crate::resource::Resource::viewed) reads it as "no
    /// detail page declared".
    pub fn is_empty(&self) -> bool {
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

    /// Render the schema read-only (GH #187): the detail page's side of the
    /// same declaration.
    ///
    /// Every field shows the record's stored value in place of its control, so
    /// a detail page reuses the field types and layout blocks a form already
    /// declares rather than a parallel infolist vocabulary. `values` is the
    /// record hydrated exactly as the edit form hydrates it
    /// ([`Resource::hydrate_form_values`](crate::resource::Resource::hydrate_form_values)):
    /// what a user reads on the page is what the form would have shown them.
    pub async fn render_readonly<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
    ) -> Result<BoxView<'a>> {
        self.render_source(
            cx,
            &RenderSource {
                values,
                errors: &HashMap::new(),
                mode: Mode::View,
            },
        )
        .await
    }

    /// Rewrite submitted values into their fields' stored spelling (GH #192).
    ///
    /// Runs after validation and before a record fn sees the map, so a typed
    /// field's `Display` — not the browser's spelling — is what gets written.
    /// That is what makes an untouched edit round-trip: the form hydrates a
    /// stored value, the browser echoes it, and this puts back the same string
    /// the record fn would have produced rather than a re-spelling of it.
    ///
    /// A `Select` takes its trimmed submission (GH #297): its presence rule and
    /// its option-existence check both read `value.trim()`, so the trimmed
    /// value is the one that passed — writing the untrimmed spelling would
    /// store a value no rule authorised.
    ///
    /// A value the caller has not validated cannot be normalised, so a parse
    /// failure here leaves the submission untouched and reports nothing: it is
    /// unreachable from the handlers (validation refuses it first), and a
    /// silent rewrite would hide a bypass rather than surface it. A field with
    /// no submission keeps its absence — an update writes only present keys.
    ///
    /// An **empty** submission is left empty for the same reason (GH #192): a
    /// typed column has no spelling for "no value" — `""` is not an `i64` and
    /// not a `Timestamp` — so inventing one here would put a value in a record
    /// the user never gave. Empty is the presence rule's business, which is
    /// where the panel already answers it: `.required()` refuses it inline, and
    /// an optional typed field reaches its record fn as `""` for the record fn's
    /// own default. The record fn therefore reads a typed field through the same
    /// "present or absent" check it uses for any other optional column.
    pub fn normalize_values(&self, values: &mut HashMap<String, String>) {
        for (name, input) in self.text_inputs() {
            let Some(submitted) = values.get(&name) else {
                continue;
            };
            if submitted.trim().is_empty() {
                continue;
            }
            if let Ok(normalized) = input.normalize(submitted) {
                values.insert(name, normalized);
            }
        }
        for name in self.select_inputs().keys() {
            let Some(submitted) = values.get(name).cloned() else {
                continue;
            };
            let trimmed = submitted.trim();
            if trimmed.len() != submitted.len() {
                values.insert(name.clone(), trimmed.to_string());
            }
        }
    }

    /// Append another schema's nodes after this one's (GH #191).
    ///
    /// [`Schema::new`] composes through `IntoSchema`, whose tuple form stops at
    /// four nodes; a derived embedded form has one control per leaf column and
    /// composes nested values, so it builds its schema by appending instead. The
    /// nodes keep their order, so a form reads in declaration order either way.
    pub fn extend(mut self, other: Schema) -> Schema {
        self.nodes.extend(other.nodes);
        // The same guard `Schema::new` runs: a derived form is built by
        // appending, so this is the only check for the shapes `new` cannot see.
        self.assert_unique_field_names();
        self
    }

    /// Render with pre-filled values and inline errors.
    pub async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        self.render_source(
            cx,
            &RenderSource {
                values,
                errors,
                mode: Mode::Form,
            },
        )
        .await
    }

    /// The one node walk: every node renders its static form.
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

    /// Collect field names for validation (TextInput + Textarea + Select + FileUpload).
    pub fn field_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for node in &self.nodes {
            for_each_field(node, &mut |n| match n {
                Node::TextInput(f) => out.push(f.field_name().to_string()),
                Node::Textarea(f) => out.push(f.field_name().to_string()),
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
    /// handlers own (`csrf_token`, `clear_<field>`, `keep_<field>`) are
    /// stripped before the record fns run (GH #148), so a generic impl cannot
    /// promote those either; `confirm`/`ids` are only read, never written.
    /// Callers should reject or ignore the rest (at least `debug_assert!` in
    /// tests); handler keys must be filtered by the caller before calling this.
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

    /// Every leaf `pick` selects, keyed by field name — the one walk behind the
    /// per-kind accessors (GH #209). `pick` answers a node's `(field name,
    /// leaf)`, or `None` when the node is not that kind.
    fn leaves<T>(&self, pick: impl Fn(&Node) -> Option<(&str, T)>) -> HashMap<String, T> {
        let mut map = HashMap::new();
        for node in &self.nodes {
            for_each_field(node, &mut |n| {
                if let Some((name, leaf)) = pick(n) {
                    map.insert(name.to_string(), leaf);
                }
            });
        }
        map
    }

    /// Every [`TextInput`] this schema declares, keyed by field name.
    pub fn text_inputs(&self) -> HashMap<String, TextInput> {
        self.leaves(|n| match n {
            Node::TextInput(f) => Some((f.field_name(), (**f).clone())),
            _ => None,
        })
    }

    /// Every [`Select`] this schema declares, keyed by field name.
    pub fn select_inputs(&self) -> HashMap<String, Select> {
        self.leaves(|n| match n {
            Node::Select(f) => Some((f.field_name(), (**f).clone())),
            _ => None,
        })
    }

    /// Every [`FileUpload`] this schema declares, keyed by field name.
    pub fn file_uploads(&self) -> HashMap<String, FileUpload> {
        self.leaves(|n| match n {
            Node::FileUpload(f) => Some((f.field_name(), (**f).clone())),
            _ => None,
        })
    }

    /// Whether this schema (including nested Section/Group/Grid/Repeater/Tabs)
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
    ///
    /// A field a submission hides is not validated (GH #297): an all-empty
    /// Repeater group is absent (GH #147), and a variant group the submission's
    /// discriminant does not name is not rendered by `variant.js`, so neither
    /// can fail the submit for a value the user cannot see.
    pub fn validate(&self, values: &HashMap<String, String>) -> HashMap<String, Vec<String>> {
        // Classify the groups first (GH #147, GH #297): an all-empty group is
        // "absent" — an untouched group submits empty strings (or omits the
        // keys), both treated as absent — so its inner inputs must not fail
        // the submit for any requiredness. A `required` repeater answers with
        // its one label-keyed error instead, and required repeaters nested
        // inside an absent group are suppressed with it. A partially filled
        // group (any inner value non-empty) enforces inner `required` as
        // usual. Whitespace-only values count as empty, matching the
        // codebase-wide trim convention. A variant group the submission's
        // discriminant does not name is hidden with its subtree, which is what
        // makes validation agree with the render (GH #297).
        let mut errors: HashMap<String, Vec<String>> = HashMap::new();
        let mut skip: HashSet<String> = HashSet::new();
        walk_absent_groups(&self.nodes, values, &mut skip, &mut errors, false);
        // One walk, one match per node (`validate_leaf`): the single place a
        // field kind joins validation (GH #209).
        for node in &self.nodes {
            for_each_field(node, &mut |n| {
                let Some((name, errs)) = validate_leaf(n, values) else {
                    return;
                };
                if !skip.contains(name) && !errs.is_empty() {
                    errors.insert(name.to_string(), errs);
                }
            });
        }
        errors
    }

    /// Field names a submission leaves out of validation (GH #147, GH #297):
    /// the same classification `validate` uses — an all-empty Repeater group
    /// is absent, and a variant group the discriminant does not name is hidden
    /// — minus the required-group errors, which validation already reported.
    /// `check_unique` consults it so an untouched group is never unique-checked
    /// while validation calls it clean, and `validate_async` so a hidden
    /// group's select is not probed for existence.
    pub(crate) fn absent_fields(&self, values: &HashMap<String, String>) -> HashSet<String> {
        let mut skip = HashSet::new();
        let mut discarded = HashMap::new();
        walk_absent_groups(&self.nodes, values, &mut skip, &mut discarded, false);
        skip
    }

    /// Async validation for Select relationship existence (tenancy-aware).
    ///
    /// A field `validate` skipped is skipped here too (GH #297): an absent
    /// repeater group or a hidden variant group holds no value the user can
    /// see, so its select must not be probed for existence.
    pub async fn validate_async(
        &self,
        cx: &Cx,
        values: &HashMap<String, String>,
    ) -> HashMap<String, Vec<String>> {
        let mut errors = self.validate(values);
        let absent = self.absent_fields(values);
        for (name, sel) in self.select_inputs() {
            if errors.contains_key(&name) || absent.contains(&name) {
                continue;
            }
            if sel.relationship.is_some() || !sel.options_static.is_empty() {
                let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
                if !val.trim().is_empty() {
                    let async_errs = sel.validate_async(cx, val).await;
                    // validate_async returns required errs plus existence; we already did required,
                    // so filter.
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

    /// GH #297: a `Select`'s presence and option checks read `value.trim()`,
    /// so the trimmed spelling is the one validation authorises. Normalisation
    /// writes exactly that value, and a padded value no option matches is still
    /// refused rather than trimmed into one.
    #[tokio::test]
    async fn a_select_stores_the_value_its_check_authorised() {
        let cx = topcoat::context::CxTestBuilder::new().build();
        let schema = Schema::new(
            Select::r#for(DummyUser::fields().name())
                .options(vec!["red".to_string(), "blue".to_string()]),
        );

        let mut values = HashMap::new();
        values.insert("name".to_string(), "  red ".to_string());
        assert!(
            schema.validate_async(&cx, &values).await.is_empty(),
            "the option check reads the trimmed value"
        );
        schema.normalize_values(&mut values);
        assert_eq!(
            values.get("name").map(String::as_str),
            Some("red"),
            "the stored value is the one the check authorised"
        );

        let mut invalid = HashMap::new();
        invalid.insert("name".to_string(), "  re d ".to_string());
        assert!(
            !schema.validate_async(&cx, &invalid).await.is_empty(),
            "trimming does not turn a non-option into one"
        );
    }

    /// GH #191: a derived form is built by appending, so `extend` carries the
    /// same duplicate-name guard `Schema::new` does (GH #100).
    #[test]
    #[should_panic(expected = "duplicate field name 'name'")]
    fn extend_keeps_the_duplicate_field_guard() {
        let input = || TextInput::r#for(DummyUser::fields().name());
        let _ = Schema::empty()
            .extend(Schema::new(input()))
            .extend(Schema::new(input()));
    }
}
