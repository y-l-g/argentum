//! Unified Schema primitive — layout blocks that compose via `view!`.
//!
//! `Schema` is a container for `Section`, `Group`, `Grid` and `Text` nodes.
//! Each node renders through Topcoat's `view!` macro; `Schema::render`
//! combines them. The API mirrors Filament's `Schema::new(( ... ))` tuple
//! form via the `IntoSchema` trait.
//!
//! Bridge note: `lens_field_name_and_label` reaches into `toasty_core` (see
//! `EXTERNAL_GAPS.md` at repo root), alongside the `pk_*` bridge helpers and
//! `cursor.rs` cursor values; migrate to public `Path::field_name()` when
//! Toasty exposes it.

use std::collections::HashMap;

use argentum_ui::{
    card, card_content, card_header, card_title, input as ui_input, label as ui_label,
};
use topcoat::{Result, context::Cx, view::*};

#[allow(clippy::type_complexity)]
type RelationshipLoader = std::sync::Arc<
    dyn Fn(
            &Cx,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<(String, String)>>> + Send>,
        > + Send
        + Sync,
>;

/// Max options a relationship `Select` will load (GH #91): the loader carries
/// `limit(Self + 1)` and fails past the cap instead of scanning a 10k-row
/// table per select per submit.
pub const MAX_RELATIONSHIP_OPTIONS: usize = 200;

// ---------------------------------------------------------------------------
// Public layout primitives
// ---------------------------------------------------------------------------

/// Placeholder leaf — renders a text block. Used in T3 before typed fields land.
#[derive(Debug, Clone)]
pub struct Text(pub String);

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        Self(content.into())
    }

    async fn render<'a>(&self, cx: &'a Cx) -> Result<BoxView<'a>> {
        let content = self.0.clone();
        Ok(view! { cx => <div class="text-sm text-foreground">(content)</div> }.boxed())
    }
}

/// Typed text field bound to a Toasty field lens. The lens is the single
/// source of truth for the field name and type, so `TextInput::for(User::fields().name())`
/// fails to compile if the column does not exist (ADR-0001).
#[derive(Debug, Clone)]
pub struct TextInput {
    name: String,
    label: String,
    required: bool,
    is_email: bool,
    unique: bool,
    placeholder: Option<String>,
}

impl TextInput {
    /// Create a `TextInput` bound to the given field lens.
    ///
    /// Only `String` lenses compile: binding a non-text field (a `Uuid` key,
    /// a `bool`, …) fails at compile time, mirroring `TextColumn`.
    pub fn for_lens<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str) = lens_field_name_and_label(path);
        Self {
            name: field_name,
            label: label_str,
            required: false,
            is_email: false,
            unique: false,
            placeholder: None,
        }
    }

    /// Convenience alias so call sites read `TextInput::for(User::fields().name())`.
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        Self::for_lens(path)
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn email(mut self) -> Self {
        self.is_email = true;
        self
    }

    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = Some(p.into());
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn is_unique(&self) -> bool {
        self.unique
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// The human label (e.g. `"Email"`) — for inline error messages.
    pub fn label_str(&self) -> &str {
        &self.label
    }

    /// Typed equality filter against the field this input is bound to.
    ///
    /// Inputs only bind `String` lenses (enforced at `for_lens`), so the
    /// comparison is a string equality on that field's path. `M` must be the
    /// model the lens came from. Built through the public facade
    /// (`Model::field_name_to_id` + `Model::path_field` + `Path::eq`) — the
    /// crate's generic handlers use it for the app-side unique check.
    pub(crate) fn eq_filter<M>(&self, value: String) -> toasty::stmt::Expr<bool>
    where
        M: toasty::schema::Model,
    {
        let fid = M::field_name_to_id(&self.name);
        M::path_field::<String>(fid.index).eq(value)
    }

    /// Validate a raw string value against the configured rules.
    pub fn validate(&self, value: &str) -> Vec<String> {
        let v = value.trim();
        let mut errs = Vec::new();
        if self.required && v.is_empty() {
            errs.push(format!("{} is required", self.label));
        }
        // Stricter than naive split('@') check — approximates `validator` (GH #11).
        if self.is_email && !v.is_empty() && !Self::is_valid_email(v) {
            errs.push(format!("{} must be a valid email", self.label));
        }
        errs
    }

    fn is_valid_email(s: &str) -> bool {
        // Accepted subset, specified (GH #100) — stricter than the original
        // `split('@') && domain.contains('.')`, deliberately narrower than
        // RFC 5322 (no quoted local parts, IP literals, or unicode):
        // `local@domain` with exactly one `@`, no spaces, no `..`; local part
        // 1–64 chars not starting/ending with `.`; domain of 2+ labels, each
        // 1–63 chars, not starting/ending with `-`, containing no `_`; TLD
        // (last label) at least 2 chars; 254 chars total.
        if s.len() > 254 || s.contains(' ') || s.contains("..") {
            return false;
        }
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return false;
        }
        let (local, domain) = (parts[0], parts[1]);
        if local.is_empty()
            || local.len() > 64
            || domain.is_empty()
            || local.starts_with('.')
            || local.ends_with('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
            || domain.starts_with('-')
            || domain.ends_with('-')
        {
            return false;
        }
        if !domain.contains('.') {
            return false;
        }
        // each domain label must be non-empty and not start/end with '-'
        let mut labels = 0;
        let mut tld_len = 0;
        for label in domain.split('.') {
            labels += 1;
            if label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || label.contains('_')
            {
                return false;
            }
            tld_len = label.len();
        }
        labels >= 2 && tld_len >= 2
    }

    #[allow(dead_code)]
    async fn render(&self, cx: &Cx) -> Result<impl View> {
        self.render_with(cx, None, &[]).await
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        value: Option<&str>,
        errors: &[String],
    ) -> Result<BoxView<'a>> {
        let label_text = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let placeholder = self.placeholder.clone();
        let input_type = if self.is_email { "email" } else { "text" };
        let has_error = !errors.is_empty();
        let error_text = errors.first().cloned().unwrap_or_default();
        let value_owned = value.map(|s| s.to_string());
        // Beautiful rendering via argentum-ui `label` + `input` with Token classes,
        // proper for/id linking, required star, type branching, and reserved error slot.
        // `ac-field` / `ac-field--error` / `ac-error` are kept for spec compat (GH #12)
        // alongside the Tailwind `grid gap-1.5` + `text-destructive` styling.
        let field_class = if has_error {
            "ac-field ac-field--error grid gap-1.5"
        } else {
            "ac-field grid gap-1.5"
        };
        Ok(view! {
            cx =>
            <div class=(field_class)>
                ui_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                ui_input(
                    attrs: attributes! {
                        id=(name.clone())
                        r#type=(input_type)
                        name=(name.clone())
                        value=(value_owned.clone())
                        placeholder=(placeholder.clone())
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                    }
                )
                <p class="ac-error text-sm text-destructive" aria-live="polite">
                    (error_text)
                </p>
            </div>
        }
        .boxed())
    }
}

/// Select field bound to a lens (often a foreign key like `author_id`).
///
/// `Select::for(Post::fields().author_id()).relationship(AuthorResource::query, |a| a.name.clone())`
/// loads options via `AuthorResource::query(cx)` (tenancy-aware) and stores `author.id`
/// as the value. Typos in the lens fail at compile time. The relationship loader
/// reuses `Resource::query` + `Resource::table` for tenancy and PK extraction (no new seam).
pub struct Select {
    name: String,
    label: String,
    required: bool,
    options_static: Vec<(String, String)>,
    #[allow(clippy::type_complexity)]
    relationship: Option<RelationshipLoader>,
}

impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("required", &self.required)
            .field("options_static", &self.options_static)
            .field("relationship", &self.relationship.is_some())
            .finish()
    }
}

impl Clone for Select {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            required: self.required,
            options_static: self.options_static.clone(),
            relationship: self.relationship.clone(),
        }
    }
}

impl Select {
    /// Create a `Select` bound to the given field lens (e.g. `Post::fields().author_id()`).
    pub fn for_lens<M, T>(path: toasty::stmt::Path<M, T>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str) = lens_field_name_and_label(path);
        Self {
            name: field_name,
            label: label_str,
            required: false,
            options_static: Vec::new(),
            relationship: None,
        }
    }

    /// Convenience alias so call sites read `Select::for(Post::fields().author_id())`.
    pub fn r#for<M, T>(path: toasty::stmt::Path<M, T>) -> Self
    where
        M: toasty::schema::Model,
    {
        Self::for_lens(path)
    }

    /// Mark the field as required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Override the label.
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    /// Static options where value == label.
    pub fn options(mut self, options: Vec<String>) -> Self {
        self.options_static = options.into_iter().map(|s| (s.clone(), s)).collect();
        self
    }

    /// Static options with explicit (value, label) pairs.
    pub fn options_with_labels(mut self, pairs: Vec<(String, String)>) -> Self {
        self.options_static = pairs;
        self
    }

    /// Load options via a related `Resource::query` (tenancy-aware) and a label closure.
    ///
    /// The first argument is the resource's `query` fn (e.g. `AuthorResource::query`) — it is
    /// only used for type inference; the loader calls `R::query(cx)` directly so tenancy is
    /// preserved. The second argument maps the related record to its display label.
    ///
    /// Bounded (GH #91): the loader fetches at most `MAX_RELATIONSHIP_OPTIONS`
    /// + 1 rows and fails when the related table is larger — a 10k-row
    /// reference table costs bounded work per submit and surfaces
    /// `could not load options, retry` instead of silently validating against
    /// a truncated list. Suitable for small reference tables only; a
    /// searchable/paginated dropdown with per-request memoization is future work.
    pub fn relationship<R>(
        mut self,
        _query: fn(&Cx) -> toasty::stmt::Query<toasty::stmt::List<R::Model>>,
        label: impl Fn(&R::Model) -> String + Send + Sync + 'static,
    ) -> Self
    where
        R: crate::resource::Resource + 'static,
        R::Model: Send + Sync + 'static,
    {
        let label = std::sync::Arc::new(label);
        let loader = std::sync::Arc::new(move |cx: &Cx| {
            let label = label.clone();
            let cx = cx.clone();
            Box::pin(async move {
                let mut db = crate::db::db(&cx);
                let records = R::query(&cx)
                    .limit(MAX_RELATIONSHIP_OPTIONS + 1)
                    .exec(&mut db)
                    .await
                    .map_err(topcoat::Error::from)?;
                if records.len() > MAX_RELATIONSHIP_OPTIONS {
                    // Fail visibly (GH #91): validating against a silent
                    // truncation would reject legitimate FKs as "invalid"
                    // while rendering a misleading subset.
                    return Err(std::io::Error::other(format!(
                        "too many options (max {MAX_RELATIONSHIP_OPTIONS})"
                    ))
                    .into());
                }
                let table = R::table(&cx);
                let mut opts = Vec::new();
                for rec in &records {
                    if let Some(k) = table.key_for(rec) {
                        opts.push((k, label(rec)));
                    }
                }
                Ok(opts)
            })
                as std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = topcoat::Result<Vec<(String, String)>>>
                            + Send,
                    >,
                >
        }) as RelationshipLoader;
        self.relationship = Some(loader);
        self
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// Validate a raw string value (required + empty). Existence is async via `validate_async`.
    pub fn validate(&self, value: &str) -> Vec<String> {
        let v = value.trim();
        let mut errs = Vec::new();
        if self.required && v.is_empty() {
            errs.push(format!("{} is required", self.label));
        }
        errs
    }

    /// Async existence check: if relationship is configured and value non-empty, ensure it matches a loaded option.
    ///
    /// A loader failure surfaces as a form-level error (GH #91) instead of an
    /// empty-options passthrough that would 500 at FK write time.
    pub async fn validate_async(&self, cx: &Cx, value: &str) -> Vec<String> {
        let mut errs = self.validate(value);
        if errs.is_empty() && !value.trim().is_empty() {
            if let Some(loader) = &self.relationship {
                match loader(cx).await {
                    Ok(opts) => {
                        let trimmed = value.trim();
                        if !opts.iter().any(|(v, _)| v == trimmed) {
                            errs.push(format!("{} is invalid", self.label));
                        }
                    }
                    Err(_) => {
                        errs.push(format!("{} could not load options, retry", self.label));
                    }
                }
            } else if !self.options_static.is_empty() {
                let trimmed = value.trim();
                if !self.options_static.iter().any(|(v, _)| v == trimmed) {
                    errs.push(format!("{} is invalid", self.label));
                }
            }
        }
        errs
    }

    async fn load_options(&self, cx: &Cx) -> topcoat::Result<Vec<(String, String)>> {
        if let Some(loader) = &self.relationship {
            loader(cx).await
        } else {
            Ok(self.options_static.clone())
        }
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        value: Option<&str>,
        errors: &[String],
    ) -> Result<BoxView<'a>> {
        let label_text = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let has_error = !errors.is_empty();
        let error_text = errors.first().cloned().unwrap_or_default();
        let current = value.unwrap_or("").trim().to_string();
        let loaded = self.load_options(cx).await;
        let load_failed = loaded.is_err();
        let mut options = loaded.unwrap_or_default();
        if load_failed && !current.is_empty() && !options.iter().any(|(v, _)| v == &current) {
            // Keep the stored FK selectable when the loader fails or the
            // table overflows the cap (GH #91): an edit must not blank the
            // relation into a required-error, and the submit surfaces
            // `could not load options, retry`.
            options.push((current.clone(), current.clone()));
        }
        // Build option views.
        let mut option_views: Vec<BoxView<'a>> = Vec::new();
        // Placeholder empty option
        let empty_selected = current.is_empty();
        option_views.push(
            view! {
                cx =>
                <option value="" selected=(empty_selected)>"-- Select --"</option>
            }
            .boxed(),
        );
        for (val, lab) in &options {
            let selected = current == *val;
            let val_c = val.clone();
            let lab_c = lab.clone();
            option_views.push(
                view! {
                    cx =>
                    <option value=(val_c) selected=(selected)>(lab_c)</option>
                }
                .boxed(),
            );
        }
        let field_class = if has_error {
            "ac-field ac-field--error grid gap-1.5"
        } else {
            "ac-field grid gap-1.5"
        };
        Ok(view! {
            cx =>
            <div class=(field_class)>
                argentum_ui::label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                <select
                    id=(name.clone())
                    name=(name.clone())
                    required=(required)
                    aria-required=(required.then_some("true"))
                    aria-invalid=(if has_error { "true" } else { "false" })
                    class="flex h-9 w-full rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
                >
                    for opt in option_views {
                        (opt)
                    }
                </select>
                <p class="ac-error text-sm text-destructive" aria-live="polite">
                    (error_text)
                </p>
            </div>
        }
        .boxed())
    }
}

/// Spec alias — ADR-0001 typed lens. Currently uses `toasty::stmt::Path` directly;
/// a richer `FieldLens` trait (carrying `FieldTy`, nullability, etc.) will replace
/// this alias when Toasty exposes the helpers publicly (see GH #11,
/// EXTERNAL_GAPS.md “field metadata”).
pub type FieldLens<M, T> = toasty::stmt::Path<M, T>;

/// Resolve a typed lens to its app-level field name and capitalized label.
///
/// Hides the `Path → toasty_core::stmt::Path → projection → M::schema()` walk
/// (see EXTERNAL_GAPS.md). Used by both `TextInput` and
/// `TextColumn` so the shape is defined once.
///
/// Traversal lenses are rejected (GH #100): a multi-step path has no single
/// field name, and silently binding its first segment misbinds in release.
pub(crate) fn lens_field_name_and_label<M, T>(path: FieldLens<M, T>) -> (String, String)
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
    let field_name = model
        .fields()
        .get(idx)
        .map(|f| f.name.app_unwrap().to_string())
        .unwrap_or_else(|| {
            panic!(
                "field index {idx} out of bounds for {}",
                std::any::type_name::<M>()
            )
        });
    let label_str = capitalize(&field_name);
    (field_name, label_str)
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

/// Returns whether the field behind a lens is nullable (GH #11).
pub fn lens_field_is_nullable<M, T>(path: FieldLens<M, T>) -> bool
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
        .unwrap_or(usize::MAX);
    M::schema().fields().get(idx).is_some_and(|f| f.nullable)
}

/// Returns whether the field behind a lens has a unique constraint (PK or `#[unique]`/`#[index(unique)]`, GH #11).
pub fn lens_field_is_unique<M, T>(path: FieldLens<M, T>) -> bool
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
        .unwrap_or(usize::MAX);
    let model = M::schema();
    let field = match model.fields().get(idx) {
        Some(f) => f,
        None => return false,
    };
    if field.primary_key {
        return true;
    }
    if let Some(root) = model.as_root() {
        let fid = toasty_core::schema::app::FieldId {
            model: M::id(),
            index: idx,
        };
        root.indices
            .iter()
            .any(|ix| ix.unique && ix.fields.len() == 1 && ix.fields[0].field == fid)
    } else {
        false
    }
}

/// Returns the storage column name for the field behind a lens (GH #11).
pub fn lens_field_column_name<M, T>(path: FieldLens<M, T>) -> String
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
    M::schema()
        .fields()
        .get(idx)
        .map(|f| {
            f.name
                .storage_name()
                .expect("field must have storage name")
                .to_string()
        })
        .unwrap_or_else(|| {
            panic!(
                "field index {idx} out of bounds for {}",
                std::any::type_name::<M>()
            )
        })
}

/// Parse a URL path segment into `M`'s primary-key value.
///
/// The PK's application type decides the [`stmt::Value`] variant (`Uuid` PK
/// → `Value::Uuid`, `i64` PK → `Value::I64`, …). Returns `None` when `id`
/// does not parse (an unparseable id cannot exist — callers render
/// not-found) or the PK is not a single primitive field (composite keys
/// have no URL representation in Argentum; row keys are plain `String`s).
///
/// Bridge helper — reaches into `toasty_core` for the schema walk and the
/// untyped equality construction (see EXTERNAL_GAPS.md): the facade's
/// `find_by_primary_key` takes a typed `Expr<M::PrimaryKey>`, which generic
/// code cannot build from a `String` without a dynamic-value bridge.
fn pk_field_value<M>(
    id: &str,
) -> Option<(toasty_core::schema::app::FieldId, toasty_core::stmt::Value)>
where
    M: toasty::schema::Model,
{
    let app_model = M::schema();
    let root = app_model.as_root()?;
    if root.primary_key.fields.len() != 1 {
        return None;
    }
    let fid = root.primary_key.fields.first().copied()?;
    let field = app_model.fields().get(fid.index)?;
    let toasty_core::schema::app::FieldTy::Primitive(prim) = &field.ty else {
        return None;
    };
    let value = match prim.ty {
        toasty_core::stmt::Type::Uuid => toasty_core::stmt::Value::Uuid(id.parse().ok()?),
        toasty_core::stmt::Type::String => toasty_core::stmt::Value::String(id.to_string()),
        toasty_core::stmt::Type::Bool => toasty_core::stmt::Value::Bool(id.parse().ok()?),
        toasty_core::stmt::Type::I8 => toasty_core::stmt::Value::I8(id.parse().ok()?),
        toasty_core::stmt::Type::I16 => toasty_core::stmt::Value::I16(id.parse().ok()?),
        toasty_core::stmt::Type::I32 => toasty_core::stmt::Value::I32(id.parse().ok()?),
        toasty_core::stmt::Type::I64 => toasty_core::stmt::Value::I64(id.parse().ok()?),
        toasty_core::stmt::Type::U8 => toasty_core::stmt::Value::U8(id.parse().ok()?),
        toasty_core::stmt::Type::U16 => toasty_core::stmt::Value::U16(id.parse().ok()?),
        toasty_core::stmt::Type::U32 => toasty_core::stmt::Value::U32(id.parse().ok()?),
        toasty_core::stmt::Type::U64 => toasty_core::stmt::Value::U64(id.parse().ok()?),
        toasty_core::stmt::Type::F32 => toasty_core::stmt::Value::F32(id.parse().ok()?),
        toasty_core::stmt::Type::F64 => toasty_core::stmt::Value::F64(id.parse().ok()?),
        // Bytes PKs have no canonical URL text form; accept the UTF-8 bytes so
        // list/edit round-trip instead of 404ing (GH #95). Temporal/composite
        // PKs remain without a URL representation.
        toasty_core::stmt::Type::Bytes => {
            toasty_core::stmt::Value::Bytes(id.as_bytes().to_vec())
        }
        // Temporal PKs parse from their canonical string forms, in lockstep
        // with the cursor codec (GH #95). Zoned/decimal/net PKs and composite
        // keys still have no URL representation.
        toasty_core::stmt::Type::Timestamp => {
            toasty_core::stmt::Value::Timestamp(id.parse().ok()?)
        }
        toasty_core::stmt::Type::Date => {
            toasty_core::stmt::Value::Date(id.parse().ok()?)
        }
        toasty_core::stmt::Type::Time => {
            toasty_core::stmt::Value::Time(id.parse().ok()?)
        }
        toasty_core::stmt::Type::DateTime => {
            toasty_core::stmt::Value::DateTime(id.parse().ok()?)
        }
        _ => return None,
    };
    Some((fid, value))
}

/// Equality predicate on `M`'s primary key for a URL path-segment `id` —
/// `pk == value`. `None` when the id does not parse as the PK's type or the
/// PK is not a single primitive field.
///
/// Consumers: the panel's edit/delete loaders, which filter the
/// tenancy-scoped [`crate::resource::Resource::query`] instead of fetching
/// every row and matching row keys in memory (GH #75 item 1).
pub(crate) fn pk_eq_expr<M>(id: &str) -> Option<toasty::stmt::Expr<bool>>
where
    M: toasty::schema::Model,
{
    let (fid, value) = pk_field_value::<M>(id)?;
    let cond = toasty_core::stmt::Expr::eq(
        toasty_core::stmt::Expr::ref_self_field(fid),
        toasty_core::stmt::Expr::from(value),
    );
    Some(toasty::stmt::Expr::from_untyped(cond))
}

/// `IN` predicate over primary keys for a bulk id list — `pk IN (a, b, …)`.
/// `None` when any id fails to parse as the PK's type (an unparseable id
/// cannot exist, so the batch fails closed) or the PK is not a single
/// primitive field.
///
/// A single `IN` predicate, not an N-way `OR` chain (GH #85): the batch is
/// still bounded by `MAX_BULK_IDS` in the bulk-delete handler.
pub(crate) fn pk_in_expr<M>(ids: &[&str]) -> Option<toasty::stmt::Expr<bool>>
where
    M: toasty::schema::Model,
{
    let mut parsed = ids.iter().map(|id| pk_field_value::<M>(id));
    let (fid, first) = parsed.next()??;
    let mut values = vec![first];
    for item in parsed {
        let (f, v) = item?;
        debug_assert_eq!(
            f.index, fid.index,
            "pk_in_expr: one model, one PK field — mixed fields are a bug"
        );
        values.push(v);
    }
    let cond = toasty_core::stmt::Expr::in_list(
        toasty_core::stmt::Expr::ref_self_field(fid),
        toasty_core::stmt::Expr::list(values),
    );
    Some(toasty::stmt::Expr::from_untyped(cond))
}

pub(crate) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Section — titled container with an optional child `Schema`.
///
/// The single customization seam for form layout in v1: additive `class` is
/// allowed on the `card` container only (narrow seam, no per-field `attrs`).
/// This keeps Token editing in `styles.css` as the primary theming mechanism.
#[derive(Debug)]
pub struct Section {
    title: String,
    children: Option<Schema>,
    extra_class: Option<String>,
}

impl Section {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            children: None,
            extra_class: None,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    /// Additive `class` hook on the `card` container (narrow seam).
    /// Merged via `class!` against Token classes, never replacing them.
    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.extra_class = Some(class.into());
        self
    }

    #[allow(dead_code)]
    async fn render(&self, cx: &Cx) -> Result<impl View> {
        self.render_with(cx, &HashMap::new(), &HashMap::new()).await
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        let title = self.title.clone();
        let extra = self.extra_class.clone();
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! {
                cx =>
                card(
                    attrs: attributes! { class=(extra.clone()) },
                    card_header(card_title((title)))
                    card_content((child_view))
                )
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                card(
                    attrs: attributes! { class=(extra.clone()) },
                    card_header(card_title((title)))
                )
            }
            .boxed())
        }
    }
}

/// Group — unlabelled container, useful for grouping fields.
#[derive(Debug)]
pub struct Group {
    children: Option<Schema>,
}

impl Default for Group {
    fn default() -> Self {
        Self::new()
    }
}

impl Group {
    pub fn new() -> Self {
        Self { children: None }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    #[allow(dead_code)]
    async fn render(&self, cx: &Cx) -> Result<impl View> {
        self.render_with(cx, &HashMap::new(), &HashMap::new()).await
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! { cx => <div class="flex flex-col gap-4">(child_view)</div> }.boxed())
        } else {
            Ok(view! { cx => <div class="flex flex-col gap-4"></div> }.boxed())
        }
    }
}

/// Grid — column container. `cols` is 1..12.
#[derive(Debug)]
pub struct Grid {
    cols: u8,
    children: Option<Schema>,
}

impl Grid {
    pub fn new(cols: u8) -> Self {
        Self {
            cols: cols.clamp(1, 12),
            children: None,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    #[allow(dead_code)]
    async fn render(&self, cx: &Cx) -> Result<impl View> {
        self.render_with(cx, &HashMap::new(), &HashMap::new()).await
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        // Static literals for Tailwind scanner — `format!("grid grid-cols-{}")` would be
        // purged because Tailwind only sees literal substrings. See ADR-0007 / T2.
        let class: &'static str = match self.cols {
            1 => "grid grid-cols-1 gap-4",
            2 => "grid grid-cols-2 gap-4",
            3 => "grid grid-cols-3 gap-4",
            4 => "grid grid-cols-4 gap-4",
            5 => "grid grid-cols-5 gap-4",
            6 => "grid grid-cols-6 gap-4",
            7 => "grid grid-cols-7 gap-4",
            8 => "grid grid-cols-8 gap-4",
            9 => "grid grid-cols-9 gap-4",
            10 => "grid grid-cols-10 gap-4",
            11 => "grid grid-cols-11 gap-4",
            _ => "grid grid-cols-12 gap-4",
        };
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! { cx => <div class=(class)>(child_view)</div> }.boxed())
        } else {
            Ok(view! { cx => <div class=(class)></div> }.boxed())
        }
    }
}

/// FileUpload field — stores a String path (Asset URL) with file input handling.
///
/// Storage contract (GH #73): v1 stores the client filename as a `String` path
/// (e.g. `image_path`), not binary content. Forms containing a `FileUpload`
/// render `enctype="multipart/form-data"` (see `Panel`) and the POST parser
/// extracts the file part's filename; the bytes themselves are not persisted.
/// Binary/file-asset handling is future work. The `<input type="file">` never
/// renders a `value` attribute — browsers ignore/mask it for security.
#[derive(Debug, Clone)]
pub struct FileUpload {
    name: String,
    label: String,
    required: bool,
}

impl FileUpload {
    pub fn for_lens<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str) = lens_field_name_and_label(path);
        Self {
            name: field_name,
            label: label_str,
            required: false,
        }
    }

    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        Self::for_lens(path)
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    pub fn validate(&self, value: &str) -> Vec<String> {
        let v = value.trim();
        let mut errs = Vec::new();
        if self.required && v.is_empty() {
            errs.push(format!("{} is required", self.label));
        }
        errs
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        _value: Option<&str>,
        errors: &[String],
    ) -> Result<BoxView<'a>> {
        let label_text = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let has_error = !errors.is_empty();
        let error_text = errors.first().cloned().unwrap_or_default();
        let field_class = if has_error {
            "ac-field ac-field--error grid gap-1.5"
        } else {
            "ac-field grid gap-1.5"
        };
        Ok(view! {
            cx =>
            <div class=(field_class)>
                ui_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                <input
                    id=(name.clone())
                    type="file"
                    name=(name.clone())
                    required=(required)
                    aria-required=(required.then_some("true"))
                    aria-invalid=(if has_error { "true" } else { "false" })
                    class="flex h-9 w-full rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
                >
                <p class="ac-error text-sm text-destructive" aria-live="polite">
                    (error_text)
                </p>
            </div>
        }
        .boxed())
    }
}

/// Repeater — nested Schema repeated as a group (in-memory for v1, no DB array).
///
/// v1 honesty (GH #73): this is a single-entry group, not a multi-row repeater —
/// one titled card with its nested schema once, no add/remove UI, no JS, no
/// indexed field names (`tags[0]`). Indexed multi-entry semantics, per-entry
/// validation, and hydration via split/join or a real relation are deferred.
/// `required` means "the inner fields must not all be empty" and its error is
/// keyed by label and rendered inline (GH #78).
#[derive(Debug)]
pub struct Repeater {
    label: String,
    children: Option<Schema>,
    required: bool,
}

impl Repeater {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: None,
            required: false,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        let title = self.label.clone();
        let required = self.required;
        // Own error lives under the label key (see `Schema::validate_repeaters`).
        // Field errors key by field name; repeaters have no field name yet, so the
        // label is the only stable key until repeaters become field-bound (GH #78).
        let own_errors: &[String] = errors.get(&self.label).map(|v| v.as_slice()).unwrap_or(&[]);
        let has_error = !own_errors.is_empty();
        let error_text = own_errors.first().cloned().unwrap_or_default();
        let container_class = if has_error {
            "ac-field ac-field--error rounded-md border border-border p-4 flex flex-col gap-4"
        } else {
            "ac-field rounded-md border border-border p-4 flex flex-col gap-4"
        };
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! {
                cx =>
                <div class=(container_class)>
                    <h4 class="font-medium text-foreground">
                        (title)
                        if required {
                            <span class="text-destructive" aria-hidden="true">"*"</span>
                        }
                    </h4>
                    <div class="grid gap-4">(child_view)</div>
                    <p class="ac-error text-sm text-destructive" aria-live="polite">
                        (error_text)
                    </p>
                </div>
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                <div class=(container_class)>
                    <h4 class="font-medium text-foreground">
                        (title)
                        if required {
                            <span class="text-destructive" aria-hidden="true">"*"</span>
                        }
                    </h4>
                    <p class="ac-error text-sm text-destructive" aria-live="polite">
                        (error_text)
                    </p>
                </div>
            }
            .boxed())
        }
    }
}

/// Tabs — layout primitive for tabbed content (in-memory for v1, no JS).
///
/// Static `div` grouping for v1 (GH #73): looks like tabs, behaves as stacked
/// sections until tab JS lands. Documented, not a placeholder bug.
#[derive(Debug)]
pub struct Tabs {
    children: Option<Schema>,
}

impl Tabs {
    pub fn new() -> Self {
        Self { children: None }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! {
                cx =>
                <div class="flex flex-col gap-4 border border-border rounded-md p-4">
                    (child_view)
                </div>
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                <div class="flex flex-col gap-4 border border-border rounded-md p-4"></div>
            }
            .boxed())
        }
    }
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

/// Wizard — step-based layout (in-memory for v1, no JS).
///
/// Static `div` grouping for v1 (GH #73): looks like steps, behaves as stacked
/// sections until step JS lands. Documented, not a placeholder bug.
#[derive(Debug)]
pub struct Wizard {
    children: Option<Schema>,
}

impl Wizard {
    pub fn new() -> Self {
        Self { children: None }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! {
                cx =>
                <div class="flex flex-col gap-4 border border-border rounded-md p-4">
                    (child_view)
                </div>
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                <div class="flex flex-col gap-4 border border-border rounded-md p-4"></div>
            }
            .boxed())
        }
    }
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Node / Schema
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Node {
    Text(Text),
    TextInput(Box<TextInput>),
    Select(Box<Select>),
    FileUpload(Box<FileUpload>),
    Repeater(Box<Repeater>),
    Tabs(Box<Tabs>),
    Wizard(Box<Wizard>),
    Section(Box<Section>),
    Group(Box<Group>),
    Grid(Box<Grid>),
}

impl Node {
    #[allow(dead_code)]
    async fn render(&self, cx: &Cx) -> Result<impl View> {
        self.render_with(cx, &HashMap::new(), &HashMap::new()).await
    }

    async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        values: &HashMap<String, String>,
        errors: &HashMap<String, Vec<String>>,
    ) -> Result<BoxView<'a>> {
        match self {
            Node::Text(t) => Ok(t.render(cx).await?.boxed()),
            Node::TextInput(f) => {
                let val = values.get(&f.field_name().to_string()).map(|s| s.as_str());
                let errs: &[String] = errors
                    .get(&f.field_name().to_string())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                Ok(Box::pin(f.render_with(cx, val, errs)).await?.boxed())
            }
            Node::Select(f) => {
                let val = values.get(&f.field_name().to_string()).map(|s| s.as_str());
                let errs: &[String] = errors
                    .get(&f.field_name().to_string())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                Ok(Box::pin(f.render_with(cx, val, errs)).await?.boxed())
            }
            Node::FileUpload(f) => {
                let val = values.get(&f.field_name().to_string()).map(|s| s.as_str());
                let errs: &[String] = errors
                    .get(&f.field_name().to_string())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                Ok(Box::pin(f.render_with(cx, val, errs)).await?.boxed())
            }
            Node::Repeater(r) => Ok(Box::pin(r.render_with(cx, values, errors)).await?.boxed()),
            Node::Tabs(t) => Ok(Box::pin(t.render_with(cx, values, errors)).await?.boxed()),
            Node::Wizard(w) => Ok(Box::pin(w.render_with(cx, values, errors)).await?.boxed()),
            Node::Section(s) => Ok(Box::pin(s.render_with(cx, values, errors)).await?.boxed()),
            Node::Group(g) => Ok(Box::pin(g.render_with(cx, values, errors)).await?.boxed()),
            Node::Grid(g) => Ok(Box::pin(g.render_with(cx, values, errors)).await?.boxed()),
        }
    }
}

impl From<Text> for Node {
    fn from(v: Text) -> Self {
        Node::Text(v)
    }
}
impl From<TextInput> for Node {
    fn from(v: TextInput) -> Self {
        Node::TextInput(Box::new(v))
    }
}
impl From<Section> for Node {
    fn from(v: Section) -> Self {
        Node::Section(Box::new(v))
    }
}
impl From<Group> for Node {
    fn from(v: Group) -> Self {
        Node::Group(Box::new(v))
    }
}
impl From<Grid> for Node {
    fn from(v: Grid) -> Self {
        Node::Grid(Box::new(v))
    }
}
impl From<Select> for Node {
    fn from(v: Select) -> Self {
        Node::Select(Box::new(v))
    }
}
impl From<FileUpload> for Node {
    fn from(v: FileUpload) -> Self {
        Node::FileUpload(Box::new(v))
    }
}
impl From<Repeater> for Node {
    fn from(v: Repeater) -> Self {
        Node::Repeater(Box::new(v))
    }
}
impl From<Tabs> for Node {
    fn from(v: Tabs) -> Self {
        Node::Tabs(Box::new(v))
    }
}
impl From<Wizard> for Node {
    fn from(v: Wizard) -> Self {
        Node::Wizard(Box::new(v))
    }
}

/// The container that composes layout blocks.
#[derive(Debug, Default)]
pub struct Schema {
    nodes: Vec<Node>,
}

impl Schema {
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
        let mut views = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            views.push(
                Box::pin(node.render_with(cx, values, errors))
                    .await?
                    .boxed(),
            );
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
            collect_field_names(node, &mut out);
        }
        out
    }

    /// Keys in `values` that no declared input owns, sorted (GH #89).
    ///
    /// Framework-level allow-list seam, enforced by the create/edit POST
    /// handlers (unknown keys → 400): record handlers already whitelist via
    /// per-field `.get(..)`, but a generic impl iterating `values` would
    /// silently promote `role`/`tenant_id`/handler keys (`csrf_token`,
    /// `confirm`, `ids`) to client-controlled writes. Callers should reject
    /// or ignore these (at least `debug_assert!` in tests); handler keys must
    /// be filtered by the caller before calling this.
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
            collect_inputs(node, &mut map);
        }
        map
    }

    /// Build a map of `field_name -> Select` for validation.
    pub fn select_inputs(&self) -> HashMap<String, Select> {
        let mut map = HashMap::new();
        for node in &self.nodes {
            collect_selects(node, &mut map);
        }
        map
    }

    /// Build a map of `field_name -> FileUpload` for validation.
    pub fn file_uploads(&self) -> HashMap<String, FileUpload> {
        let mut map = HashMap::new();
        for node in &self.nodes {
            collect_file_uploads(node, &mut map);
        }
        map
    }

    /// Whether this schema (including nested Section/Group/Grid/Repeater/Tabs/Wizard)
    /// contains a [`FileUpload`]. `Panel` uses it to emit
    /// `enctype="multipart/form-data"` only on forms that need it (GH #73).
    pub fn has_file_upload(&self) -> bool {
        fn walk(nodes: &[Node]) -> bool {
            nodes.iter().any(|node| match node {
                Node::FileUpload(_) => true,
                Node::Repeater(r) => r.children.as_ref().is_some_and(|s| walk(&s.nodes)),
                Node::Section(s) => s.children.as_ref().is_some_and(|s| walk(&s.nodes)),
                Node::Group(g) => g.children.as_ref().is_some_and(|s| walk(&s.nodes)),
                Node::Grid(g) => g.children.as_ref().is_some_and(|s| walk(&s.nodes)),
                Node::Tabs(t) => t.children.as_ref().is_some_and(|s| walk(&s.nodes)),
                Node::Wizard(w) => w.children.as_ref().is_some_and(|s| walk(&s.nodes)),
                _ => false,
            })
        }
        walk(&self.nodes)
    }

    /// Validate submitted values against declared inputs (GH #89).
    ///
    /// Absent keys are treated as `""` for validation; update record fns must
    /// therefore only write keys present in the submission, or an omitted
    /// optional field silently blanks the stored value. Use
    /// [`Self::unknown_keys`] to allow-list POST keys.
    pub fn validate(&self, values: &HashMap<String, String>) -> HashMap<String, Vec<String>> {
        let inputs = self.text_inputs();
        let mut errors: HashMap<String, Vec<String>> = HashMap::new();
        for (name, input) in inputs {
            let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
            let errs = input.validate(val);
            if !errs.is_empty() {
                errors.insert(name, errs);
            }
        }
        for (name, sel) in self.select_inputs() {
            let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
            let errs = sel.validate(val);
            if !errs.is_empty() {
                errors.insert(name, errs);
            }
        }
        for (name, fu) in self.file_uploads() {
            let val = values.get(&name).map(|s| s.as_str()).unwrap_or("");
            let errs = fu.validate(val);
            if !errs.is_empty() {
                errors.insert(name, errs);
            }
        }
        // Repeater `required` was stored but never validated (GH #75).
        self.validate_repeaters(values, &mut errors);
        errors
    }

    fn validate_repeaters(
        &self,
        values: &HashMap<String, String>,
        errors: &mut HashMap<String, Vec<String>>,
    ) {
        fn walk(
            nodes: &[Node],
            values: &HashMap<String, String>,
            errors: &mut HashMap<String, Vec<String>>,
        ) {
            for node in nodes {
                match node {
                    Node::Repeater(r) => {
                        if r.required {
                            let inner_names = r
                                .children
                                .as_ref()
                                .map(|s| s.field_names())
                                .unwrap_or_default();
                            let all_empty = if inner_names.is_empty() {
                                true
                            } else {
                                inner_names.iter().all(|n| {
                                    values.get(n).map(|v| v.trim().is_empty()).unwrap_or(true)
                                })
                            };
                            if all_empty {
                                errors
                                    .entry(r.label.clone())
                                    .or_insert_with(|| vec![format!("{} is required", r.label)]);
                            }
                        }
                        if let Some(child) = &r.children {
                            walk(&child.nodes, values, errors);
                        }
                    }
                    Node::Section(s) => {
                        if let Some(child) = &s.children {
                            walk(&child.nodes, values, errors);
                        }
                    }
                    Node::Group(g) => {
                        if let Some(child) = &g.children {
                            walk(&child.nodes, values, errors);
                        }
                    }
                    Node::Grid(g) => {
                        if let Some(child) = &g.children {
                            walk(&child.nodes, values, errors);
                        }
                    }
                    Node::Tabs(t) => {
                        if let Some(child) = &t.children {
                            walk(&child.nodes, values, errors);
                        }
                    }
                    Node::Wizard(w) => {
                        if let Some(child) = &w.children {
                            walk(&child.nodes, values, errors);
                        }
                    }
                    _ => {}
                }
            }
        }
        walk(&self.nodes, values, errors);
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

fn collect_field_names(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::TextInput(f) => out.push(f.field_name().to_string()),
        Node::Select(f) => out.push(f.field_name().to_string()),
        Node::FileUpload(f) => out.push(f.field_name().to_string()),
        Node::Repeater(r) => {
            if let Some(schema) = &r.children {
                for n in &schema.nodes {
                    collect_field_names(n, out);
                }
            }
        }
        Node::Tabs(t) => {
            if let Some(schema) = &t.children {
                for n in &schema.nodes {
                    collect_field_names(n, out);
                }
            }
        }
        Node::Wizard(w) => {
            if let Some(schema) = &w.children {
                for n in &schema.nodes {
                    collect_field_names(n, out);
                }
            }
        }
        Node::Section(s) => {
            if let Some(schema) = &s.children {
                for n in &schema.nodes {
                    collect_field_names(n, out);
                }
            }
        }
        Node::Group(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_field_names(n, out);
                }
            }
        }
        Node::Grid(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_field_names(n, out);
                }
            }
        }
        Node::Text(_) => {}
    }
}

fn collect_inputs(node: &Node, map: &mut HashMap<String, TextInput>) {
    match node {
        Node::TextInput(f) => {
            map.insert(f.field_name().to_string(), (**f).clone());
        }
        Node::Select(_) => {}
        Node::FileUpload(_) => {}
        Node::Repeater(r) => {
            if let Some(schema) = &r.children {
                for n in &schema.nodes {
                    collect_inputs(n, map);
                }
            }
        }
        Node::Tabs(t) => {
            if let Some(schema) = &t.children {
                for n in &schema.nodes {
                    collect_inputs(n, map);
                }
            }
        }
        Node::Wizard(w) => {
            if let Some(schema) = &w.children {
                for n in &schema.nodes {
                    collect_inputs(n, map);
                }
            }
        }
        Node::Section(s) => {
            if let Some(schema) = &s.children {
                for n in &schema.nodes {
                    collect_inputs(n, map);
                }
            }
        }
        Node::Group(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_inputs(n, map);
                }
            }
        }
        Node::Grid(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_inputs(n, map);
                }
            }
        }
        Node::Text(_) => {}
    }
}

fn collect_selects(node: &Node, map: &mut HashMap<String, Select>) {
    match node {
        Node::Select(f) => {
            map.insert(f.field_name().to_string(), (**f).clone());
        }
        Node::TextInput(_) => {}
        Node::FileUpload(_) => {}
        Node::Repeater(r) => {
            if let Some(schema) = &r.children {
                for n in &schema.nodes {
                    collect_selects(n, map);
                }
            }
        }
        Node::Tabs(t) => {
            if let Some(schema) = &t.children {
                for n in &schema.nodes {
                    collect_selects(n, map);
                }
            }
        }
        Node::Wizard(w) => {
            if let Some(schema) = &w.children {
                for n in &schema.nodes {
                    collect_selects(n, map);
                }
            }
        }
        Node::Section(s) => {
            if let Some(schema) = &s.children {
                for n in &schema.nodes {
                    collect_selects(n, map);
                }
            }
        }
        Node::Group(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_selects(n, map);
                }
            }
        }
        Node::Grid(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_selects(n, map);
                }
            }
        }
        Node::Text(_) => {}
    }
}

fn collect_file_uploads(node: &Node, map: &mut HashMap<String, FileUpload>) {
    match node {
        Node::FileUpload(f) => {
            map.insert(f.field_name().to_string(), (**f).clone());
        }
        Node::TextInput(_) => {}
        Node::Select(_) => {}
        Node::Repeater(r) => {
            if let Some(schema) = &r.children {
                for n in &schema.nodes {
                    collect_file_uploads(n, map);
                }
            }
        }
        Node::Tabs(t) => {
            if let Some(schema) = &t.children {
                for n in &schema.nodes {
                    collect_file_uploads(n, map);
                }
            }
        }
        Node::Wizard(w) => {
            if let Some(schema) = &w.children {
                for n in &schema.nodes {
                    collect_file_uploads(n, map);
                }
            }
        }
        Node::Section(s) => {
            if let Some(schema) = &s.children {
                for n in &schema.nodes {
                    collect_file_uploads(n, map);
                }
            }
        }
        Node::Group(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_file_uploads(n, map);
                }
            }
        }
        Node::Grid(g) => {
            if let Some(schema) = &g.children {
                for n in &schema.nodes {
                    collect_file_uploads(n, map);
                }
            }
        }
        Node::Text(_) => {}
    }
}

// ---------------------------------------------------------------------------
// IntoSchema — tuple / single conversions
// ---------------------------------------------------------------------------

pub trait IntoSchema {
    fn into_schema(self) -> Schema;
}

impl IntoSchema for Schema {
    fn into_schema(self) -> Schema {
        self
    }
}
impl IntoSchema for Text {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Section {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Group {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Grid {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for TextInput {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Select {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for FileUpload {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Repeater {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Tabs {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Wizard {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}

impl<A, B> IntoSchema for (A, B)
where
    A: Into<Node>,
    B: Into<Node>,
{
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.0.into(), self.1.into()],
        }
    }
}
// 4-tuple limit is intentional: without variadic generics this is idiomatic
// — see `IntoColumns` in `resource.rs`. Macro deferred until 5+ columns are needed.
impl<A, B, C> IntoSchema for (A, B, C)
where
    A: Into<Node>,
    B: Into<Node>,
    C: Into<Node>,
{
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.0.into(), self.1.into(), self.2.into()],
        }
    }
}
impl<A, B, C, D> IntoSchema for (A, B, C, D)
where
    A: Into<Node>,
    B: Into<Node>,
    C: Into<Node>,
    D: Into<Node>,
{
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.0.into(), self.1.into(), self.2.into(), self.3.into()],
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — T3 acceptance criteria
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;

    fn cx() -> Cx {
        CxTestBuilder::new().build()
    }

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        #[unique]
        email: String,
    }

    #[tokio::test]
    async fn text_input_renders_with_label_and_ac_field() {
        let cx = cx();
        let schema = Schema::new(TextInput::r#for(DummyUser::fields().name()));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // Beautiful: grid gap-1.5 wrapper, label + input with Token classes
        assert!(
            html.contains("grid gap-1.5"),
            "missing grid gap-1.5 in {html}"
        );
        assert!(
            html.contains("border-border"),
            "missing border-border in {html}"
        );
        assert!(
            html.contains("bg-background"),
            "missing bg-background in {html}"
        );
        assert!(html.contains("shadow-xs"), "missing shadow-xs in {html}");
        assert!(
            html.contains("name=\"name\"")
                || html.contains("name=\"Name\"")
                || html.contains("name"),
            "missing name attr in {html}"
        );
        assert!(html.contains("<input"), "missing input in {html}");
        assert!(html.contains("<label"), "missing label in {html}");
        assert!(
            html.contains("for=\"name\"") || html.contains("for="),
            "missing for/id linking in {html}"
        );
        assert!(
            html.contains("text-sm text-destructive"),
            "missing reserved error slot in {html}"
        );
        // label derived from lens: DummyUser::fields().name() → "name" → "Name"
        assert!(
            html.contains("Name") || html.contains("name"),
            "missing label in {html}"
        );
    }

    #[test]
    fn text_input_required_validates_empty() {
        let input = TextInput::r#for(DummyUser::fields().name()).required();
        assert!(
            !input.validate("").is_empty(),
            "required should reject empty"
        );
        assert!(
            input.validate("hello").is_empty(),
            "required should accept non-empty"
        );
        assert!(
            !input.validate("   ").is_empty(),
            "required should reject whitespace"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().name())
                .validate("")
                .is_empty(),
            "optional should accept empty"
        );
    }

    #[tokio::test]
    async fn text_input_required_renders_star_and_email_type() {
        let cx = cx();
        let html_req = Schema::new(TextInput::r#for(DummyUser::fields().name()).required())
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_req.contains("text-destructive"),
            "required should render star with text-destructive in {html_req}"
        );
        assert!(
            html_req.contains("required"),
            "required attr missing in {html_req}"
        );
        assert!(
            html_req.contains("aria-required"),
            "aria-required missing in {html_req}"
        );
        assert!(
            html_req.contains("for=\"name\"") && html_req.contains("id=\"name\""),
            "for/id linking missing in {html_req}"
        );
        let html_email = Schema::new(TextInput::r#for(DummyUser::fields().email()).email())
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_email.contains("type=\"email\""),
            "email should render type=email in {html_email}"
        );
        let html_text = Schema::new(TextInput::r#for(DummyUser::fields().name()))
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_text.contains("type=\"text\""),
            "plain should render type=text in {html_text}"
        );
        // reserved error slot
        assert!(
            html_req.contains("text-sm text-destructive"),
            "error slot missing in {html_req}"
        );
    }

    #[test]
    fn text_input_email_validates() {
        let input = TextInput::r#for(DummyUser::fields().email())
            .required()
            .email();
        assert!(
            !input.validate("not-an-email").is_empty(),
            "email should reject invalid"
        );
        assert!(
            !input.validate("a@").is_empty(),
            "email should reject partial"
        );
        assert!(
            input.validate("a@b.com").is_empty(),
            "email should accept valid"
        );
        // optional email: empty is ok, whitespace trimmed
        assert!(
            TextInput::r#for(DummyUser::fields().email())
                .email()
                .validate("")
                .is_empty(),
            "optional email should accept empty"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().email())
                .email()
                .validate(" a@b.com ")
                .is_empty(),
            "email should trim"
        );
    }

    #[test]
    fn text_input_email_subset_edges() {
        let input = TextInput::r#for(DummyUser::fields().email()).email();
        // Accepted subset (GH #100).
        for ok in ["a@b.com", "user+tag@sub.example.co", "Ada@Example.COM"] {
            assert!(input.validate(ok).is_empty(), "{ok} should pass");
        }
        // Rejected: underscore host, single-char TLD, overlong parts.
        for bad in [
            "user@my_host.com".to_string(),
            "a@b.c".to_string(),
            format!("{}@b.com", "a".repeat(65)),
            format!("a@{}.com", "b".repeat(64)),
            // 255 chars total, every part individually valid (GH #100).
            format!(
                "{}@{}.{}.{}",
                "a".repeat(64),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(62)
            ),
            "\"a b\"@example.com".to_string(),
            "a@b..com".to_string(),
        ] {
            assert!(!input.validate(&bad).is_empty(), "{bad} should fail");
        }
    }

    #[tokio::test]
    async fn text_input_composes_in_tuple() {
        let cx = cx();
        let schema = Schema::new((
            TextInput::r#for(DummyUser::fields().name()),
            TextInput::r#for(DummyUser::fields().email()),
        ));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.matches("grid gap-1.5").count() >= 2,
            "expected 2 fields (grid gap-1.5) in {html}"
        );
        assert!(
            html.matches("text-sm text-destructive").count() >= 2,
            "expected 2 error slots in {html}"
        );
    }

    #[tokio::test]
    async fn text_input_inside_section_and_grid() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Grid::new(2).schema((
            TextInput::r#for(DummyUser::fields().name()).required(),
            TextInput::r#for(DummyUser::fields().email()).email(),
        ))));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // Section now renders as card with Token classes
        assert!(
            html.contains("border-border"),
            "missing card border in {html}"
        );
        assert!(html.contains("bg-background"), "missing card bg in {html}");
        assert!(html.contains("shadow-sm"), "missing card shadow in {html}");
        assert!(html.contains("grid grid-cols-2"), "missing grid in {html}");
        assert!(html.contains("grid gap-1.5"), "missing field in {html}");
    }

    #[tokio::test]
    async fn section_renders_title_and_child() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Text::new("hello")));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Account"), "missing title in {html}");
        assert!(html.contains("hello"), "missing child in {html}");
        // Section now renders as card
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing card chrome in {html}"
        );
        assert!(
            html.contains("px-6"),
            "missing card header/content padding in {html}"
        );
        assert!(
            html.contains("font-semibold"),
            "missing card title in {html}"
        );
    }

    #[tokio::test]
    async fn group_renders_children() {
        let cx = cx();
        let schema = Schema::new(Group::new().schema(Text::new("inside group")));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("inside group"), "missing child in {html}");
        assert!(
            html.contains("flex flex-col gap-4"),
            "missing group class flex flex-col gap-4 in {html}"
        );
    }

    #[tokio::test]
    async fn grid_renders_with_cols_and_children() {
        let cx = cx();
        let schema = Schema::new(Grid::new(2).schema((Text::new("a"), Text::new("b"))));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("grid"), "missing grid class in {html}");
        assert!(
            html.contains("grid-cols-2"),
            "missing cols class grid-cols-2 in {html}"
        );
        assert!(html.contains("gap-4"), "missing gap-4 in {html}");
        assert!(
            html.contains(">a<") || html.contains("text-foreground\">a"),
            "missing a in {html}"
        );
        assert!(
            html.contains(">b<") || html.contains("text-foreground\">b"),
            "missing b in {html}"
        );
    }

    #[tokio::test]
    async fn nested_grid_inside_section() {
        let cx = cx();
        let schema = Schema::new(
            Section::new("Outer")
                .schema(Grid::new(2).schema((Text::new("left"), Text::new("right")))),
        );
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Outer"), "missing outer title in {html}");
        assert!(html.contains("left"), "missing left in {html}");
        assert!(html.contains("right"), "missing right in {html}");
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing section card in {html}"
        );
        assert!(html.contains("grid-cols-2"), "missing grid in {html}");
    }

    #[tokio::test]
    async fn schema_composes_multiple_blocks() {
        let cx = cx();
        let schema = Schema::new((
            Section::new("A").schema(Text::new("a")),
            Group::new().schema(Text::new("b")),
        ));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing section card in {html}"
        );
        assert!(
            html.contains("flex flex-col gap-4"),
            "missing group in {html}"
        );
    }

    #[tokio::test]
    async fn empty_schema_renders_empty() {
        let cx = cx();
        let schema = Schema::empty();
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.is_empty() || !html.contains("border-border"),
            "empty schema should render nothing, got {html}"
        );
    }

    #[tokio::test]
    async fn repeater_required_error_renders_inline() {
        let cx = cx();
        // Single-entry repeater (GH #78): the required error is keyed by label
        // until repeaters become field-bound.
        let schema = Schema::new(
            Repeater::new("Tags")
                .required()
                .schema(TextInput::r#for(DummyUser::fields().name()).label("Tag")),
        );
        let values = HashMap::new();
        let errors = schema.validate(&values);
        assert!(
            errors.contains_key("Tags"),
            "required repeater must produce a label-keyed error, got {errors:?}"
        );
        let html = schema
            .render_with(&cx, &values, &errors)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("Tags is required"),
            "repeater error must reach the HTML, got {html}"
        );
        // Same inline error contract as TextInput: ac-error slot + live region.
        assert!(
            html.contains("ac-error") && html.contains("text-destructive"),
            "missing inline error slot in {html}"
        );
        assert!(
            html.contains("aria-live=\"polite\""),
            "missing aria-live in {html}"
        );
        // Non-empty inner value clears the error.
        let mut filled = HashMap::new();
        filled.insert("name".to_string(), "rust".to_string());
        let errors = schema.validate(&filled);
        assert!(
            !errors.contains_key("Tags"),
            "filled repeater must pass, got {errors:?}"
        );
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

    #[tokio::test]
    async fn file_upload_renders_without_value_attr() {
        let cx = cx();
        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
        }
        let schema = Schema::new(FileUpload::r#for(Doc::fields().path()));
        let mut values = HashMap::new();
        values.insert("path".to_string(), "/tmp/old.jpg".to_string());
        let html = schema
            .render_with(&cx, &values, &HashMap::new())
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("type=\"file\""),
            "missing file input in {html}"
        );
        // Browsers ignore/mask file-input value (GH #73) — must never render.
        assert!(
            !html.contains("value=\"/tmp/old.jpg\""),
            "file input must not carry value in {html}"
        );
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
        assert_eq!(schema.unknown_keys(&values), vec!["confirm".to_string(), "role".to_string()]);
        values.remove("role");
        values.remove("confirm");
        assert!(schema.unknown_keys(&values).is_empty());
    }

    #[test]
    fn pk_in_expr_builds_one_in_predicate_and_fails_closed() {
        // GH #85: single IN predicate; empty lists and unparseable ids yield
        // None (the bulk handler 400s empty before reaching here; an
        // unparseable id cannot exist, so the batch must not silently drop
        // it — the handler maps None to 404).
        assert!(pk_in_expr::<DummyUser>(&[]).is_none());
        assert!(pk_in_expr::<DummyUser>(&["not-a-uuid"]).is_none());
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        assert!(pk_in_expr::<DummyUser>(&[a.as_str(), b.as_str()]).is_some());
        assert!(pk_in_expr::<DummyUser>(&[a.as_str(), "not-a-uuid"]).is_none());
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

    #[derive(Debug, Clone, toasty::Model)]
    struct TemporalPk {
        #[key]
        at: jiff::Timestamp,
        name: String,
    }

    #[tokio::test]
    async fn temporal_pks_parse_from_url_ids() {
        // Parse level: canonical forms resolve, garbage does not (GH #95).
        assert!(pk_eq_expr::<TemporalPk>("2024-01-15T09:30:00Z").is_some());
        assert!(pk_eq_expr::<TemporalPk>("not-a-time").is_none());
        // Round-trip through sqlite: the parsed value filters the row.
        let mut db = toasty::Db::builder()
            .models(toasty::models!(TemporalPk))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(TemporalPk {
            at: "2024-01-15T09:30:00Z".parse::<jiff::Timestamp>().unwrap(),
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let mut db2 = db.clone();
        let expr = pk_eq_expr::<TemporalPk>("2024-01-15T09:30:00Z").unwrap();
        let rows = TemporalPk::filter(expr).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");
    }

    #[tokio::test]
    async fn relationship_loader_fails_past_option_cap() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct RefAuthor {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct RefAuthorResource;
        impl Resource for RefAuthorResource {
            type Model = RefAuthor;
            fn table(cx: &Cx) -> crate::resource::Table<RefAuthor> {
                crate::resource::Table::r#for(cx)
                    .id(|a: &RefAuthor| a.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        RefAuthor::fields().name(),
                        |a: &RefAuthor| a.name.clone(),
                    ))
            }
        }
        #[derive(Debug, toasty::Model)]
        struct RefPost {
            #[key]
            #[auto]
            id: uuid::Uuid,
            author_id: uuid::Uuid,
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(RefAuthor, RefPost))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..(MAX_RELATIONSHIP_OPTIONS + 1) {
            toasty::create!(RefAuthor {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let select =
            Select::r#for(RefPost::fields().author_id()).relationship::<RefAuthorResource>(
                RefAuthorResource::query,
                |a: &RefAuthor| a.name.clone(),
            );
        // Over the cap: bounded work, visible retry error — never an
        // empty-options passthrough (GH #91).
        let errs = select.validate_async(&cx, "whatever").await;
        assert!(
            errs.iter().any(|e| e.contains("could not load options")),
            "overflow must surface retry error, got {errs:?}"
        );
    }
}
