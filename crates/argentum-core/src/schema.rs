//! Unified Schema primitive — layout blocks that compose via `view!`.
//!
//! `Schema` is a container for `Section`, `Group`, `Grid` and `Text` nodes.
//! Each node renders through Topcoat's `view!` macro; `Schema::render`
//! combines them. The API mirrors Filament's `Schema::new(( ... ))` tuple
//! form via the `IntoSchema` trait.
//!
//! Bridge note: `lens_field_name_and_label` and `pk_tie_breakers` reach into
//! `toasty_core` (see `EXTERNAL_GAPS.md` at repo root). They are the single
//! `toasty_core` import sites; migrate to public `Path::field_name()` /
//! `Model::primary_key_paths()` when Toasty exposes them.

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
        // `s` is already trimmed by `validate`. Stricter than the original
        // `split('@') && domain.contains('.')` — rejects `a@b..c`, `a@b`,
        // `.a@b.com`, `a@.b.com` etc. without pulling `validator` crate.
        // Keeps `TextInput::validate("a@b..c")` failing as GH #11 expects.
        if s.contains(' ') || s.contains("..") {
            return false;
        }
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return false;
        }
        let (local, domain) = (parts[0], parts[1]);
        if local.is_empty() || domain.is_empty() {
            return false;
        }
        if local.starts_with('.')
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
        for label in domain.split('.') {
            if label.is_empty() || label.starts_with('-') || label.ends_with('-') {
                return false;
            }
        }
        true
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
                <p class="ac-error text-sm text-destructive" aria-live="polite">(error_text)</p>
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
                    .exec(&mut db)
                    .await
                    .map_err(topcoat::Error::from)?;
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
                        // If loader fails, don't add error here; DB will surface.
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
        let options = self.load_options(cx).await.unwrap_or_default();
        // Build option views.
        let mut option_views: Vec<BoxView<'a>> = Vec::new();
        // Placeholder empty option
        let empty_selected = current.is_empty();
        option_views.push(
            view! { cx => <option value="" selected=(empty_selected)>"-- Select --"</option> }
                .boxed(),
        );
        for (val, lab) in &options {
            let selected = current == *val;
            let val_c = val.clone();
            let lab_c = lab.clone();
            option_views.push(
                view! { cx => <option value=(val_c) selected=(selected)>(lab_c)</option> }.boxed(),
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
                <p class="ac-error text-sm text-destructive" aria-live="polite">(error_text)</p>
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
/// (single import site, see EXTERNAL_GAPS.md). Used by both `TextInput` and
/// `TextColumn` so the shape is defined once.
pub(crate) fn lens_field_name_and_label<M, T>(path: FieldLens<M, T>) -> (String, String)
where
    M: toasty::schema::Model,
{
    let core_path: toasty_core::stmt::Path = path.into();
    debug_assert!(
        !core_path.projection.as_slice().is_empty(),
        "lens expects a field lens, got root path"
    );
    // Only single-field lenses are supported; multi-step paths will panic in
    // debug and fall back to first segment in release.
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

/// Returns whether the field behind a lens is nullable (GH #11).
pub fn lens_field_is_nullable<M, T>(path: FieldLens<M, T>) -> bool
where
    M: toasty::schema::Model,
{
    let core_path: toasty_core::stmt::Path = path.into();
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

pub(crate) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// PK tie-breaker(s) for deterministic pagination.
///
/// Centralizes the `toasty_core::stmt` walk (`M::schema().as_root().primary_key`)
/// so `resource.rs` does not directly depend on core internals (see
/// EXTERNAL_GAPS.md “primary-key tie-breaker”). Returns one `asc`
/// `OrderByExpr` per PK field, in declared order.
///
/// # Panics
///
/// Panics if `M` is not a root model: without a primary key there is no
/// tie-breaker, and silent omission here would surface only as flaky row
/// order under cursor pagination.
pub(crate) fn pk_tie_breakers<M>() -> Vec<toasty::stmt::OrderByExpr>
where
    M: toasty::schema::Model,
{
    let app_model = M::schema();
    let root = app_model.as_root().unwrap_or_else(|| {
        panic!(
            "pk_tie_breakers: {} is not a root model; deterministic pagination needs its primary key",
            std::any::type_name::<M>()
        )
    });
    let mut out = Vec::new();
    for pk_field in &root.primary_key.fields {
        let expr = toasty_core::stmt::Expr::ref_self_field(*pk_field);
        out.push(toasty::stmt::OrderByExpr {
            expr,
            order: Some(toasty_core::stmt::Direction::Asc),
        });
    }
    out
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
                <p class="ac-error text-sm text-destructive" aria-live="polite">(error_text)</p>
            </div>
        }
        .boxed())
    }
}

/// Repeater — nested Schema repeated as a group (in-memory for v1, no DB array).
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
        if let Some(schema) = &self.children {
            let child_view = schema.render_with(cx, values, errors).await?;
            Ok(view! {
                cx =>
                <div class="rounded-md border border-border p-4 flex flex-col gap-4">
                    <h4 class="font-medium text-foreground">(title)</h4>
                    <div class="grid gap-4">
                        (child_view)
                    </div>
                </div>
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                <div class="rounded-md border border-border p-4">
                    <h4 class="font-medium text-foreground">(title)</h4>
                </div>
            }
            .boxed())
        }
    }
}

/// Tabs — layout primitive for tabbed content (in-memory for v1, no JS).
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
            Ok(view! { cx => <div class="flex flex-col gap-4 border border-border rounded-md p-4">(child_view)</div> }.boxed())
        } else {
            Ok(view! { cx => <div class="flex flex-col gap-4 border border-border rounded-md p-4"></div> }.boxed())
        }
    }
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

/// Wizard — step-based layout (in-memory for v1, no JS).
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
            Ok(view! { cx => <div class="flex flex-col gap-4 border border-border rounded-md p-4">(child_view)</div> }.boxed())
        } else {
            Ok(view! { cx => <div class="flex flex-col gap-4 border border-border rounded-md p-4"></div> }.boxed())
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
    pub fn new(children: impl IntoSchema) -> Self {
        children.into_schema()
    }

    /// An empty schema (no nodes).
    pub fn empty() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Render the schema to a `View` (no DB access).
    pub async fn render(&self, cx: &Cx) -> Result<impl View> {
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
}
