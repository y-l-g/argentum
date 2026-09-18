//! Field leaves — `Text`, `TextInput`, `Select`, `FileUpload`.
//!
//! Typed inputs bound to Toasty field lenses; the lens is the single
//! source of truth for the field name, label, and required default.

use argentum_ui::{
    field as ui_field, field_error as ui_field_error, field_label as ui_field_label,
    input as ui_input,
};
use topcoat::runtime::Signal;
use topcoat::{Result, context::Cx, view::*};

use super::lenses::lens_field_name_label_and_nullable;
use super::relationship::{
    OptionLoadError, RelatedPrimaryKey, RelationshipLoadFuture, RelationshipLoader, related_records,
};

/// Placeholder leaf — renders a text block. Used in T3 before typed fields land.
#[derive(Debug, Clone)]
pub struct Text(pub String);

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        Self(content.into())
    }

    pub(crate) async fn render<'a>(&self, cx: &'a Cx) -> Result<BoxView<'a>> {
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
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str, nullable) = lens_field_name_label_and_nullable(path);
        Self {
            name: field_name,
            label: label_str,
            // Non-nullable columns are required by default (GH #100): an
            // empty submit would die at the driver instead of failing
            // inline. Override with `.optional()` for nullable columns.
            required: !nullable,
            is_email: false,
            unique: false,
            placeholder: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Opt out of the non-nullable default (GH #100): for nullable columns
    /// where an empty submit is legitimate.
    pub fn optional(mut self) -> Self {
        self.required = false;
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

    /// Whether an empty submit fails validation (`required`, defaulting from
    /// column nullability per GH #100). The app-side unique check uses it to
    /// skip empty values it would never write (GH #88).
    pub(crate) fn is_required(&self) -> bool {
        self.required
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
    /// Inputs only bind `String` lenses (enforced at `r#for`), so the
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

    /// Static render: the create/edit path's control, with `value` rendered
    /// into the `value` attribute and `errors` into the error slot.
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
        // Beautiful rendering via the upstream `field` family (topcoat#420):
        // label + control + reserved error slot, the label following the
        // field's invalid state, and `aria-invalid` driving the control's
        // error border/ring. `ac-field` / `ac-field--error` / `ac-error` are
        // kept for spec compat (GH #12).
        let field_class = if has_error {
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        Ok(view! {
            cx =>
            ui_field(
                attrs: attributes! {
                    class=(field_class)
                    data-invalid=(has_error.then_some("true"))
                },
                ui_field_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                ui_input(
                    attrs: attributes! {
                        id=(name.clone())
                        type=(input_type)
                        name=(name.clone())
                        value=(value_owned.clone())
                        placeholder=(placeholder.clone())
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    }
                )
                ui_field_error(
                    attrs: attributes! {
                        id=(error_id.clone())
                        class="ac-error"
                        aria-live="polite"
                    },
                    (error_text)
                )
            )
        }
        .boxed())
    }

    /// Render this field with its value bound to `value` and its inline
    /// errors taken from the (server-rendered) `errors` (GH #154 §4).
    ///
    /// The control renders through [`bound_input`](argentum_ui::bound_input),
    /// so typing writes the signal and a shard re-render reads the typed
    /// value; the label, chrome, error slot, and `aria-invalid` state are the
    /// same as [`Self::render_with`], so a re-render updates them in place.
    pub(crate) async fn render_live_with<'a>(
        &self,
        cx: &'a Cx,
        value: &Signal<String>,
        errors: &[String],
    ) -> Result<BoxView<'a>> {
        let label_text = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let placeholder = self.placeholder.clone();
        let input_type = if self.is_email { "email" } else { "text" };
        let has_error = !errors.is_empty();
        let error_text = errors.first().cloned().unwrap_or_default();
        let field_class = if has_error {
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        let value = value.clone();
        Ok(view! {
            cx =>
            ui_field(
                attrs: attributes! {
                    class=(field_class)
                    data-invalid=(has_error.then_some("true"))
                },
                ui_field_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                argentum_ui::bound_input(
                    value: value,
                    attrs: attributes! {
                        id=(name.clone())
                        type=(input_type)
                        name=(name.clone())
                        placeholder=(placeholder.clone())
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    }
                )
                ui_field_error(
                    attrs: attributes! {
                        id=(error_id.clone())
                        class="ac-error"
                        aria-live="polite"
                    },
                    (error_text)
                )
            )
        }
        .boxed())
    }
}

/// Select field bound to a lens (often a foreign key like `author_id`).
///
/// `Select::for(Post::fields().author_id()).relationship(AuthorResource::query, |a| a.id, |a| a.name.clone())`
/// loads options via `AuthorResource::query(cx)` (tenancy-aware) and stores the
/// related record's primary key as the value. Typos in the lens fail at compile
/// time; a wrong value projection fails where the projected type differs from
/// the PK. Option values are never read from the related table's `Table::id`
/// row-key projection (GH #108); that projection stays the list's row identity
/// (DOM ids, bulk values, edit/delete URLs), not a source of FK values.
pub struct Select {
    name: String,
    label: String,
    required: bool,
    searchable: bool,
    pub(crate) options_static: Vec<(String, String)>,
    #[allow(clippy::type_complexity)]
    pub(crate) relationship: Option<RelationshipLoader>,
}

impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("required", &self.required)
            .field("searchable", &self.searchable)
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
            searchable: self.searchable,
            options_static: self.options_static.clone(),
            relationship: self.relationship.clone(),
        }
    }
}

impl Select {
    /// Create a `Select` bound to the given field lens (e.g. `Post::fields().author_id()`).
    ///
    /// Required defaults from the lens's nullability (GH #100, GH #147): a
    /// non-nullable FK (`Post::fields().author_id()`) rejects an empty submit
    /// inline instead of dying at the driver's `parse::<Uuid>("")`; opt out
    /// with `.optional()` for nullable columns. A bare `Select` over a
    /// foreign-key lens validates only presence (any value passes) — prefer
    /// [`.relationship()`](Self::relationship), which checks existence
    /// tenancy-aware, for FK fields (GH #91).
    pub fn r#for<M, T>(path: toasty::stmt::Path<M, T>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str, nullable) = lens_field_name_label_and_nullable(path);
        Self {
            name: field_name,
            label: label_str,
            required: !nullable,
            searchable: false,
            options_static: Vec::new(),
            relationship: None,
        }
    }

    /// Client-side option search (GH #91): renders a filter input above the
    /// select that narrows options by label substring (delegated JS, no
    /// re-render). Covers the bounded option set (relationship loads are
    /// capped); over-cap tables still fail visibly, and server-side option
    /// search for huge tables is future work (#150). No-JS keeps the plain
    /// select.
    pub fn searchable(mut self) -> Self {
        self.searchable = true;
        self
    }

    /// Mark the field as required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Opt out of the non-nullable default (GH #100, GH #147): for nullable
    /// columns where an empty submit is legitimate, or for a non-nullable
    /// column the form does not collect (a record fn substitutes a value —
    /// note the browser-side required attribute drops too, and an empty
    /// submit then reaches the record fn as `""`).
    pub fn optional(mut self) -> Self {
        self.required = false;
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

    /// Load options via a related `Resource::query` (tenancy-aware), a typed
    /// primary-key projection, and a label closure.
    ///
    /// The first argument is the resource's `query` fn (e.g. `AuthorResource::query`) — it is
    /// only used for type inference; the loader calls `R::query(cx)` directly so tenancy is
    /// preserved. The second argument projects each related record to the
    /// model's **primary key**: it is stringified with `Display` and becomes
    /// the `<option value>`. The third maps the record to its display label.
    ///
    /// Option values are typed PKs, never the table's row-key projection
    /// (GH #108): using `Table::id` as the option value silently stored
    /// arbitrary display strings in FK columns (or 500'd at write time when
    /// the record fn parsed them). The projection is
    /// `Fn(&R::Model) -> R::Model::PrimaryKey`, so a wrong field fails to
    /// compile where the types differ. The related PK must be a single
    /// primitive implementing `Display` — its canonical string is what
    /// round-trips through the form; composite-key and `Bytes`-key models
    /// cannot declare relationship selects (use `options_with_labels` for
    /// those). Edit forms must hydrate the FK with that same canonical string
    /// (e.g. `record.author_id.to_string()`), or the stored value renders
    /// unselected.
    ///
    /// Policy-checked (GH #108): the related resource must allow
    /// `can_view_any` for the request and, when it declares
    /// `requires_tenant`, have a resolved tenant; each loaded row is then
    /// filtered through `can_view` before its label can render. A denial
    /// fails the whole load closed: the select renders no options and not
    /// the stored value, the field shows `{label} is not available` on GET,
    /// and a submit that still carries a value fails with that message. A
    /// required denied select cannot be submitted at all (the empty control
    /// fails `required` validation first); on an optional select an
    /// untouched denied value submits empty, so record fns that must
    /// preserve an inaccessible FK should treat `""` as "leave unchanged"
    /// (the framework does not substitute it).
    ///
    /// Bounded and memoized (GH #91): the loader fetches at most one row
    /// past `MAX_RELATIONSHIP_OPTIONS` (before `can_view` filtering) and
    /// fails when the related table is larger — a 10k-row reference table
    /// costs bounded work per submit and surfaces `could not load options,
    /// retry` instead of silently validating against a truncated list.
    /// Option records are memoized per `(request, tenant)`, so any number of
    /// selects over one resource share a single load. Suitable for small
    /// reference tables only; a searchable/paginated dropdown is future
    /// work.
    pub fn relationship<R>(
        mut self,
        _query: fn(&Cx) -> toasty::stmt::Query<toasty::stmt::List<R::Model>>,
        value: impl Fn(&R::Model) -> RelatedPrimaryKey<R> + Send + Sync + 'static,
        label: impl Fn(&R::Model) -> String + Send + Sync + 'static,
    ) -> Self
    where
        R: crate::resource::Resource + 'static,
        R::Model: Send + Sync + 'static,
        RelatedPrimaryKey<R>: std::fmt::Display,
    {
        let value = std::sync::Arc::new(value);
        let label = std::sync::Arc::new(label);
        let loader = std::sync::Arc::new(move |cx: &Cx| {
            let value = value.clone();
            let label = label.clone();
            let cx = cx.clone();
            Box::pin(async move {
                let records = match related_records::<R>(&cx, crate::tenancy::tenant_id(&cx)).await
                {
                    Ok(records) => records,
                    // `#[memoize(as_ref)]` hands back a borrow, so clone the
                    // (small) error back into this future's owned result.
                    Err(err) => return Err(err.clone()),
                };
                let mut opts = Vec::new();
                for rec in records.iter() {
                    opts.push((value(rec).to_string(), label(rec)));
                }
                Ok(opts)
            }) as RelationshipLoadFuture
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
    /// empty-options passthrough that would 500 at FK write time. A policy
    /// denial (GH #108) is reported as "not available" — retrying cannot fix
    /// a permission decision, and "invalid" would misattribute it to the
    /// submitted value.
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
                    Err(OptionLoadError::Denied) => {
                        errs.push(format!("{} is not available", self.label));
                    }
                    Err(OptionLoadError::LoadFailed) => {
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

    async fn load_options(&self, cx: &Cx) -> Result<Vec<(String, String)>, OptionLoadError> {
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
        let searchable = self.searchable;
        let current = value.unwrap_or("").trim().to_string();
        let loaded = self.load_options(cx).await;
        // A policy denial (GH #108) deliberately does not re-render the
        // stored value: the related rows are not viewable, so neither is
        // their label — the submit fails closed with "not available". The
        // denial is also surfaced on GET (when the caller carries no error
        // yet): the select has no options to pick, so the empty control must
        // explain itself instead of looking like a requireable empty field.
        // A failed/over-cap load keeps the stored FK selectable (GH #91): an
        // edit must not blank the relation into a required-error, and the
        // submit surfaces `could not load options, retry`.
        let denied = matches!(&loaded, Err(OptionLoadError::Denied));
        let keep_current_value = matches!(&loaded, Err(OptionLoadError::LoadFailed));
        let mut options = loaded.unwrap_or_default();
        if keep_current_value && !current.is_empty() && !options.iter().any(|(v, _)| v == &current)
        {
            options.push((current.clone(), current.clone()));
        }
        let incoming_error = errors.first().cloned().unwrap_or_default();
        let error_text = if incoming_error.is_empty() && denied {
            format!("{} is not available", self.label)
        } else {
            incoming_error
        };
        let has_error = !errors.is_empty() || denied;
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
        // The upstream `field` family (topcoat#420) with the reserved error
        // slot kept for spec compat (GH #12). The raw control carries the
        // same `aria-invalid` error styling as the `input` primitive.
        let field_class = if has_error {
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        let filter_label = format!("Filter {label_text} options");
        Ok(view! {
            cx =>
            ui_field(
                attrs: attributes! {
                    class=(field_class)
                    data-select-filterable=""
                    data-invalid=(has_error.then_some("true"))
                },
                ui_field_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                if searchable {
                    ui_input(
                        attrs: attributes! {
                            type="search"
                            aria-label=(filter_label.clone())
                            placeholder="Filter…"
                            data-options-filter=""
                            class="h-9"
                        }
                    )
                }
                <select
                    id=(name.clone())
                    name=(name.clone())
                    required=(required)
                    aria-required=(required.then_some("true"))
                    aria-invalid=(if has_error { "true" } else { "false" })
                    aria-describedby=(has_error.then_some(error_id.clone()))
                    class="flex h-9 w-full rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive"
                >
                    for opt in option_views {
                        (opt)
                    }
                </select>
                ui_field_error(
                    attrs: attributes! {
                        id=(error_id.clone())
                        class="ac-error"
                        aria-live="polite"
                    },
                    (error_text)
                )
            )
        }
        .boxed())
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
    /// Create a `FileUpload` bound to the given field lens.
    ///
    /// Required defaults from the lens's nullability (GH #100, GH #147),
    /// same as `TextInput`/`Select`. Today the `String` lens type only binds
    /// non-nullable columns (`Option<String>` fields do not typecheck), so
    /// the default is always required and `.optional()` is the form-level
    /// opt-out; the nullability walk stays correct if the lens widens
    /// upstream (#115).
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str, nullable) = lens_field_name_label_and_nullable(path);
        Self {
            name: field_name,
            label: label_str,
            required: !nullable,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Opt out of the required default (GH #147): today `r#for` only binds
    /// non-nullable `String` columns (an `Option<String>` field is
    /// `Path<M, Option<String>>` and does not typecheck), so the default is
    /// always required and this is the only way to treat a required-backed
    /// column as form-optional — an empty submit then passes validation and
    /// the record fn decides what to store.
    pub fn optional(mut self) -> Self {
        self.required = false;
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
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        Ok(view! {
            cx =>
            ui_field(
                attrs: attributes! {
                    class=(field_class)
                    data-invalid=(has_error.then_some("true"))
                },
                ui_field_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                // The `input` primitive styles `type="file"` through its
                // `file:` classes and carries the `aria-invalid` error styling.
                ui_input(
                    attrs: attributes! {
                        id=(name.clone())
                        type="file"
                        name=(name.clone())
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    }
                )
                ui_field_error(
                    attrs: attributes! {
                        id=(error_id.clone())
                        class="ac-error"
                        aria-live="polite"
                    },
                    (error_text)
                )
            )
        }
        .boxed())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use topcoat::context::{Cx, CxTestBuilder};

    use crate::schema::Schema;

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

    /// A nullable FK, for the optional-by-default select case.
    #[derive(Debug, toasty::Model)]
    struct NullableRef {
        #[key]
        #[auto]
        id: uuid::Uuid,
        parent_id: Option<uuid::Uuid>,
    }

    /// A non-nullable foreign key, for the required-by-default FK select.
    #[derive(Debug, toasty::Model)]
    struct FkRef {
        #[key]
        #[auto]
        id: uuid::Uuid,
        author_id: uuid::Uuid,
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
        // Beautiful: the upstream field wrapper + field_label, and the input
        // with Token classes
        assert!(
            html.contains("data-slot=\"field\"") && html.contains("data-slot=\"field-label\""),
            "missing field/field-label markup in {html}"
        );
        assert!(
            html.contains("border-border"),
            "missing border-border in {html}"
        );
        assert!(
            html.contains("bg-transparent") && html.contains("focus-visible:ring-ring"),
            "missing Token input classes in {html}"
        );
        assert!(
            html.contains("name=\"name\""),
            "missing name attr in {html}"
        );
        assert!(html.contains("<input"), "missing input in {html}");
        assert!(html.contains("<label"), "missing label in {html}");
        assert!(
            html.contains("for=\"name\""),
            "missing for/id linking in {html}"
        );
        assert!(
            html.contains("text-sm text-destructive"),
            "missing reserved error slot in {html}"
        );
        // label derived from lens: DummyUser::fields().name() → "name" → "Name"
        assert!(html.contains(">Name"), "missing label in {html}");
    }

    #[tokio::test]
    async fn text_input_error_marks_the_field_invalid() {
        // topcoat#420: `aria-invalid` drives the input's error border/ring and
        // the label's destructive color; the reserved slot carries the id the
        // control describes itself with.
        let cx = cx();
        let schema = Schema::new(TextInput::r#for(DummyUser::fields().name()).required());
        let mut errors = HashMap::new();
        errors.insert("name".to_string(), vec!["name is required".to_string()]);
        let html = schema
            .render_with(&cx, &HashMap::new(), &errors)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-invalid=\"true\"") && html.contains("ac-field--error"),
            "missing invalid field state in {html}"
        );
        assert!(
            html.contains("aria-invalid=\"true\"")
                && html.contains("aria-describedby=\"name-error\""),
            "missing aria invalid/described-by in {html}"
        );
        assert!(
            html.contains("aria-invalid:border-destructive"),
            "missing error border styling in {html}"
        );
        assert!(
            html.contains("id=\"name-error\"") && html.contains("name is required"),
            "missing error slot content in {html}"
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
            !TextInput::r#for(DummyUser::fields().name())
                .validate("")
                .is_empty(),
            "non-nullable columns default to required (GH #100)"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().name())
                .optional()
                .validate("")
                .is_empty(),
            "optional should accept empty"
        );
    }

    #[test]
    fn required_default_follows_lens_nullability() {
        // GH #100: `required` defaults from the DB column, with an explicit
        // `.optional()` escape hatch. Pinned through the public constructor
        // (the standalone nullability helper was removed as dead code, GH #137).
        #[derive(Debug, toasty::Model)]
        struct NullableDoc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            nick: Option<String>,
        }
        assert!(
            Select::r#for(NullableDoc::fields().nick())
                .validate("")
                .is_empty(),
            "nullable columns default to optional"
        );
        assert!(
            !TextInput::r#for(DummyUser::fields().name())
                .validate("")
                .is_empty(),
            "String columns are non-nullable, empty must fail inline"
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
        // `r#type` would still contain the substring `type=`, so pin the
        // attribute name itself (GH #151: the raw identifier leaked into the
        // rendered HTML and made every email input a plain text input).
        assert!(
            html_email.contains("type=\"email\"") && !html_email.contains("r#type"),
            "email should render type=email, not r#type=email, in {html_email}"
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
            html_text.contains("type=\"text\"") && !html_text.contains("r#type"),
            "plain should render type=text, not r#type=text, in {html_text}"
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
                .optional()
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

    /// `Select`/`FileUpload` follow the same required-default as `TextInput`
    /// (GH #147): non-nullable lenses default required, `.optional()` opts
    /// out, `.required()` forces it back.
    #[test]
    fn select_and_file_upload_required_defaults_follow_nullability() {
        // Non-nullable String field: bare Select/FileUpload reject "".
        let select = Select::r#for(DummyUser::fields().name());
        assert!(
            select
                .validate("")
                .iter()
                .any(|e| e.contains("is required")),
            "non-nullable"
        );
        let upload = FileUpload::r#for(DummyUser::fields().name());
        assert!(
            upload
                .validate("")
                .iter()
                .any(|e| e.contains("is required")),
            "non-nullable"
        );

        // `.optional()` opts out; `.required()` forces it back on.
        assert!(
            Select::r#for(DummyUser::fields().name())
                .optional()
                .validate("")
                .is_empty()
        );
        assert!(
            FileUpload::r#for(DummyUser::fields().name())
                .optional()
                .validate("")
                .is_empty()
        );
        assert!(
            Select::r#for(DummyUser::fields().name())
                .optional()
                .required()
                .validate("")
                .iter()
                .any(|e| e.contains("is required"))
        );

        // Nullable lenses default optional (GH #100 parity).
        let select = Select::r#for(NullableRef::fields().parent_id());
        assert!(
            select.validate("").is_empty(),
            "nullable FK select defaults optional, got {:?}",
            select.validate("")
        );
        // FileUpload's lens type only binds non-nullable `String` columns
        // (a nullable field is `Option<String>`), so its required default is
        // always on today; the nullability walk keeps it correct if the lens
        // widens upstream (#115).
    }

    /// A bare `Select` over a non-nullable FK must reject an empty submit
    /// inline (GH #147): previously it passed validation and died at the
    /// driver's `parse::<Uuid>("")` as a 500.
    #[test]
    fn bare_non_nullable_fk_select_rejects_empty_inline() {
        let select = Select::r#for(FkRef::fields().author_id());
        let errs = select.validate("");
        assert!(
            errs.iter().any(|e| e.contains("is required")),
            "bare non-nullable FK must reject empty inline, got {errs:?}"
        );
        // `.optional()` opts back out.
        let errs = Select::r#for(FkRef::fields().author_id())
            .optional()
            .validate("");
        assert!(errs.is_empty(), "opt-out must clear required, got {errs:?}");
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

    #[tokio::test]
    async fn searchable_select_renders_filter_input() {
        // GH #91: opt-in client-side option search; default selects stay bare.
        let cx = CxTestBuilder::new().build();
        let plain = Select::r#for(DummyUser::fields().name()).options(vec!["a".to_string()]);
        let html = plain
            .render_with(&cx, None, &[])
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("data-options-filter"),
            "default select must stay bare, got {html}"
        );
        let searchable = Select::r#for(DummyUser::fields().name())
            .options(vec!["a".to_string()])
            .searchable();
        let html = searchable
            .render_with(&cx, None, &[])
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-options-filter"),
            "searchable select must render the filter hook, got {html}"
        );
        assert!(
            html.contains("data-select-filterable"),
            "searchable select must scope the filter, got {html}"
        );
    }
}
