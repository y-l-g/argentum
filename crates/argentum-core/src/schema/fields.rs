//! Field leaves — `Text`, `TextInput`, `Textarea`, `Select`, `FileUpload`.
//!
//! Typed inputs bound to Toasty field lenses; the lens is the single
//! source of truth for the field name, label, and required default.

use argentum_ui::{
    field as ui_field, field_content as ui_field_content, field_error as ui_field_error,
    field_label as ui_field_label, field_title as ui_field_title, input as ui_input,
    select as ui_select, textarea as ui_textarea,
};
use topcoat::runtime::Signal;
use topcoat::{Result, context::Cx, view::*};

use super::lenses::{FieldResolver, lens_field, lens_field_unique, lens_label};
use super::relationship::{
    OptionLoadError, RelatedCheck, RelatedPrimaryKey, RelationshipCheckFuture, RelationshipChecker,
    RelationshipLoadFuture, RelationshipLoader, RelationshipSearchFuture, RelationshipSearchLoader,
    related_record_check, related_records, related_records_search,
};
use super::tree::Mode;

/// How a read-only value is presented (GH #187).
///
/// Two shapes, because the difference is content, not styling: prose wraps
/// mid-word never, and an identifier (a stored path, an address) has no spaces
/// to break at, so it breaks anywhere and sets in mono. A `bool` parameter
/// carried the same decision until it read as validation metadata
/// (`self.is_email`) at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    /// Wrapping text: a title, a body, a description.
    Prose,
    /// A path, a key, an address — no spaces to break at.
    Machine,
}

/// The read-only half of a field (GH #187): the label with the record's stored
/// value under it, no control and no validation slot.
///
/// Every field type renders its view through this, so a detail page reads
/// uniformly and the one place that decides "how does a value look" lives here
/// rather than in the page handler. The label is the same `field_label` the
/// form uses, inside the same `field` family, so a field is recognisable across
/// the two pages.
///
/// An absent value and an empty one render the same, deliberately: the
/// framework stores `""` rather than NULL (GH #89), so a stored record cannot
/// tell them apart and the page must not pretend otherwise.
fn render_value<'a>(
    cx: &'a Cx,
    label: &str,
    value: Option<&str>,
    kind: ValueKind,
) -> Result<BoxView<'a>> {
    let label = label.to_string();
    let text = value.unwrap_or_default().to_string();
    let value_class = match kind {
        ValueKind::Prose => "text-sm break-words whitespace-pre-wrap",
        ValueKind::Machine => "text-sm font-mono break-all whitespace-pre-wrap",
    };
    Ok(view! {
        cx =>
        ui_field(
            attrs: attributes! { class="ac-field" },
            ui_field_content(
                ui_field_title((label))
                <div class=(value_class)>(text)</div>
            )
        )
    }
    .boxed())
}

/// Placeholder leaf — renders a text block. Used in T3 before typed fields land.
#[derive(Debug, Clone)]
pub(crate) struct Text(pub String);

impl Text {
    /// Test-only since GH #173: the placeholder leaf left the public surface
    /// (`pub(crate)`), and only the layout/tree unit tests still build one.
    #[cfg(test)]
    pub(crate) fn new(content: impl Into<String>) -> Self {
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
    ///
    /// `required` and `unique` default from the field's metadata (GH #100,
    /// GH #183): a non-nullable column is required, and a field backed by a
    /// single-field unique index is unique — so neither has to be restated by
    /// hand. Both stay overridable with `.optional()` / `.unique()`.
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let model = M::schema();
        let field = lens_field(path, &model);
        let label_str = lens_label(&field);
        let unique = lens_field_unique(&field, model.as_root_unwrap());
        Self {
            name: field.name.app_unwrap().to_string(),
            label: label_str,
            // Non-nullable columns are required by default (GH #100): an
            // empty submit would die at the driver instead of failing
            // inline. Override with `.optional()` for nullable columns.
            required: !field.nullable(),
            is_email: false,
            unique,
            placeholder: None,
        }
    }

    /// Create a `TextInput` bound to a lens inside an embedded struct or a
    /// `#[document]` (GH #185).
    ///
    /// The plain [`Self::r#for`] resolves a lens against the model alone, which
    /// is why it can only bind a top-level field: the owned `app::Model` cannot
    /// see the embedded models, so a path like `Post::fields().seo().title()`
    /// is rejected as a traversal lens. This resolves through the request's app
    /// schema instead, so the leaf arrives as its **flattened storage column**
    /// (`seo_title`) — the name the form posts and the record fn reads.
    ///
    /// An embedded leaf is never `required` by default: every column under an
    /// embedded step is storage-nullable, since only the matching enum variant
    /// writes one. Opt in with [`.required()`](Self::required).
    ///
    /// Without a `Db` in context (a bare `CxTestBuilder`) this behaves exactly
    /// like [`Self::r#for`] and rejects the traversal lens loudly, so a test
    /// cannot silently bind the wrong column.
    pub fn r#for_context<M>(cx: &Cx, path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let leaf = FieldResolver::from_cx(cx).resolve(path);
        Self {
            name: leaf.name,
            label: leaf.label,
            required: !leaf.nullable,
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

    /// Mark the field as backed by a unique constraint, which the app-side
    /// pre-check probes before the write.
    ///
    /// **Uniqueness implies presence** (GH #189): the framework stores `""`,
    /// never NULL (GH #89), so an empty value is one the index admits only
    /// once — an empty submit is refused inline as `"<Label> is required"`
    /// instead of being written, and the probe never sees it. `.optional()`
    /// does not lift that rule, whichever order the two are called in. The
    /// reasoning (and the rejected alternative) is recorded in the ADR-0010
    /// amendment of 2026-09-21.
    ///
    /// Non-`TextInput` fields declare no uniqueness (see [`Textarea::r#for`]),
    /// so nothing else changes.
    pub fn unique(mut self) -> Self {
        self.unique = true;
        // The marker carries presence itself, so `.optional().unique()` and
        // `.unique().optional()` mean the same thing.
        self.required = true;
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

    /// Whether an empty submit fails validation and the control renders as
    /// required: `required`, defaulting from column nullability per GH #100,
    /// **or** uniqueness (GH #189 — a unique field is never empty, see
    /// [`Self::unique`]). `validate` and both render paths read it, so the rule
    /// and the marker cannot disagree.
    pub(crate) fn is_required(&self) -> bool {
        self.required || self.unique
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
        // One presence rule, one predicate, shared with the render marker.
        if self.is_required() && v.is_empty() {
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
        mode: Mode,
    ) -> Result<BoxView<'a>> {
        if mode == Mode::View {
            return render_value(cx, &self.label, value, ValueKind::Machine);
        }
        let label_text = self.label.clone();
        let name = self.name.clone();
        // The marker reads the same predicate validation uses, so a unique
        // field is never refused for emptiness while rendering as optional
        // (GH #189). `self.required` alone would do exactly that.
        let required = self.is_required();
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
                if has_error {
                    ui_field_error(
                        attrs: attributes! {
                            id=(error_id.clone())
                            class="ac-error"
                            aria-live="polite"
                        },
                        (error_text)
                    )
                }
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
        // As `render_with`: validation and the marker read one predicate.
        let required = self.is_required();
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
                if has_error {
                    ui_field_error(
                        attrs: attributes! {
                            id=(error_id.clone())
                            class="ac-error"
                            aria-live="polite"
                        },
                        (error_text)
                    )
                }
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
    #[allow(clippy::type_complexity)]
    pub(crate) relationship_search: Option<RelationshipSearchLoader>,
    #[allow(clippy::type_complexity)]
    pub(crate) relationship_check: Option<RelationshipChecker>,
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
            .field("relationship_search", &self.relationship_search.is_some())
            .field("relationship_check", &self.relationship_check.is_some())
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
            relationship_search: self.relationship_search.clone(),
            relationship_check: self.relationship_check.clone(),
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
        let field = lens_field(path, &M::schema());
        Self {
            name: field.name.app_unwrap().to_string(),
            label: lens_label(&field),
            required: !field.nullable(),
            searchable: false,
            options_static: Vec::new(),
            relationship: None,
            relationship_search: None,
            relationship_check: None,
        }
    }

    /// Option search over a visible list (GH #91, GH #184) plus server-side
    /// narrowing past the cap (GH #150): renders a filter input and a
    /// suggestion listbox above the select. Typing narrows the list by label
    /// substring for bounded sets, and picks write the chosen option onto the
    /// select, which stays the form control. Past the cap it instead fetches
    /// `GET {parent_list_url}/options?field=&q=` (debounced, abort in-flight,
    /// selection preserved) and re-renders the list from the answer. Reuses the
    /// related `Table`'s declared `searchable()` columns; non-searchable
    /// selects keep the cap error. No-JS keeps the plain select (relation
    /// cannot be changed past the cap, other fields still submit).
    ///
    /// The list exists because the select cannot show filtering itself: the
    /// primitive opts into `appearance: base-select`, whose popup is browser
    /// chrome that ignores `option[hidden]`, so narrowing the select's own
    /// options is invisible (GH #184).
    ///
    /// Behavior asset: the field needs `assets/selects.js`
    /// (`argentum_ui::SELECTS_JS`, hooks `data-select-filterable` /
    /// `data-options-filter` / `data-options-combobox` / `data-options-list`),
    /// emitted by `Panel::render_document` on every document with shell assets
    /// (see ADR-0014). Without the document scripts the input is inert and the
    /// plain select keeps working.
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
    /// overflows when the related table is larger — a 10k-row reference table
    /// costs bounded work per submit. Small tables validate against the
    /// bounded set; overflowed tables surface `Overflow` (GH #150): searchable
    /// selects degrade to type-to-search with a targeted existence check,
    /// non-searchable ones keep the `could not load options, retry` error.
    /// Base option records are memoized per `(request, tenant)` so any number
    /// of selects over one resource share the load; searches and targeted
    /// checks are single bounded round-trips per call, not shared.
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
        let value_s = value.clone();
        let label_s = label.clone();
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
        let search_loader = std::sync::Arc::new(move |cx: &Cx, q: String| {
            let value = value_s.clone();
            let label = label_s.clone();
            let cx = cx.clone();
            Box::pin(async move {
                let records = match related_records_search::<R>(&cx, q).await {
                    Ok(records) => records,
                    Err(err) => return Err(err.clone()),
                };
                let mut opts = Vec::new();
                for rec in records.iter() {
                    opts.push((value(rec).to_string(), label(rec)));
                }
                Ok(opts)
            }) as RelationshipSearchFuture
        }) as RelationshipSearchLoader;
        let check_loader = std::sync::Arc::new(move |cx: &Cx, v: String| {
            let cx = cx.clone();
            Box::pin(async move {
                match related_record_check::<R>(&cx, v).await {
                    Ok(check) => Ok(check),
                    Err(err) => Err(err.clone()),
                }
            }) as RelationshipCheckFuture
        }) as RelationshipChecker;
        self.relationship = Some(loader);
        self.relationship_search = Some(search_loader);
        self.relationship_check = Some(check_loader);
        self
    }

    pub(crate) fn is_searchable(&self) -> bool {
        self.searchable
    }

    pub(crate) fn is_relationship(&self) -> bool {
        self.relationship.is_some()
    }

    /// Server-side option search for the endpoint (GH #150 D1/D5).
    ///
    /// Clamps `q`, reuses the related table's searchable columns, bounds to
    /// `MAX_RELATIONSHIP_OPTIONS`. Returns `Overflow` when the filtered set
    /// still exceeds the cap (caller renders "keep typing").
    pub(crate) async fn search_options(
        &self,
        cx: &Cx,
        q: &str,
    ) -> Result<Vec<(String, String)>, OptionLoadError> {
        if let Some(search) = &self.relationship_search {
            search(cx, q.to_string()).await
        } else if let Some(loader) = &self.relationship {
            loader(cx).await
        } else {
            Ok(self.options_static.clone())
        }
    }

    /// Targeted existence check for overflowed selects (GH #150 D4).
    async fn check_overflowed(&self, cx: &Cx, value: &str) -> Vec<String> {
        let Some(check) = &self.relationship_check else {
            return vec![format!("{} could not load options, retry", self.label)];
        };
        match check(cx, value.trim().to_string()).await {
            Ok(RelatedCheck::FoundViewable) => Vec::new(),
            Ok(RelatedCheck::FoundHidden) | Ok(RelatedCheck::NotFound) => {
                vec![format!("{} is invalid", self.label)]
            }
            Err(OptionLoadError::Denied) => {
                vec![format!("{} is not available", self.label)]
            }
            Err(OptionLoadError::LoadFailed) | Err(OptionLoadError::Overflow) => {
                vec![format!("{} could not load options, retry", self.label)]
            }
        }
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
    /// submitted value. An overflowed load (GH #150) uses the targeted check
    /// for searchable selects (legitimate FKs beyond the cap validate) and
    /// keeps the retry error for non-searchable ones.
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
                    Err(OptionLoadError::Overflow) if self.searchable => {
                        return self.check_overflowed(cx, value).await;
                    }
                    Err(OptionLoadError::Overflow) | Err(OptionLoadError::LoadFailed) => {
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
        mode: Mode,
    ) -> Result<BoxView<'a>> {
        // View mode resolves a static option label and never loads options
        // (GH #187): a detail page renders one record, so a relationship's
        // option load would be a query per page, and its scoped/denied paths
        // exist to police a *choice* the page is not offering. A relationship
        // therefore shows its stored key — the same value the column beside it
        // shows — and the detail page's own includes are what make a related
        // record readable.
        if mode == Mode::View {
            let stored = value.unwrap_or("").trim();
            let shown = self
                .options_static
                .iter()
                .find(|(v, _)| v == stored)
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| stored.to_string());
            return render_value(cx, &self.label, Some(&shown), ValueKind::Prose);
        }
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
        // A failed load keeps the stored FK selectable (GH #91): an edit must
        // not blank the relation into a required-error, and the submit
        // surfaces `could not load options, retry`. An overflowed load
        // (GH #150) also keeps the stored FK; searchable selects degrade to
        // type-to-search with a hint (no retry error), non-searchable ones
        // keep the retry path.
        let denied = matches!(&loaded, Err(OptionLoadError::Denied));
        let overflowed = matches!(&loaded, Err(OptionLoadError::Overflow));
        let overflow_searchable = overflowed && searchable && self.relationship.is_some();
        let keep_current_value = matches!(
            &loaded,
            Err(OptionLoadError::LoadFailed) | Err(OptionLoadError::Overflow)
        );
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
        // The upstream `field` family (topcoat#420): the `select` primitive
        // brings the same `aria-invalid` error styling and focus ring as the
        // `input` primitive, plus the chevron and the customizable picker —
        // the control no longer hand-rolls the input chrome.
        let field_class = if has_error {
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        let filter_label = format!("Filter {label_text} options");
        // Server fetch only past the cap (GH #150): bounded searchable sets
        // keep the client-side label-substring filter (GH #91), so the
        // `data-options-server` flag must follow the overflow state — not
        // every searchable relationship. `selects.js` branches on this flag.
        let options_field = overflow_searchable.then(|| name.clone());
        let options_server = overflow_searchable.then_some("true");
        let options_overflow = overflow_searchable.then_some("true");
        let overflow_hint = "Too many options — type to search".to_string();
        Ok(view! {
            cx =>
            ui_field(
                attrs: attributes! {
                    class=(field_class)
                    data-select-filterable=""
                    data-invalid=(has_error.then_some("true"))
                    data-options-field=(options_field)
                    data-options-server=(options_server)
                    data-options-overflow=(options_overflow)
                },
                ui_field_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                if searchable {
                    // The filter input and its suggestion list (GH #184). The
                    // list is what makes the filter visible: the native
                    // `<select>` popup is browser chrome the script cannot
                    // narrow (the primitive opts into `appearance: base-select`,
                    // where `option[hidden]` has no effect), so `selects.js`
                    // renders its own filtered list here and writes the chosen
                    // value onto the select. Without the script the input is
                    // inert and the plain select keeps working.
                    <div class="relative" data-options-combobox="">
                        ui_input(
                            attrs: attributes! {
                                type="search"
                                aria-label=(filter_label.clone())
                                placeholder="Filter…"
                                data-options-filter=""
                                class="h-9"
                                autocomplete="off"
                            }
                        )
                        <ul
                            data-options-list=""
                            role="listbox"
                            aria-label=(filter_label.clone())
                            hidden=""
                            class="absolute z-20 mt-1 max-h-60 w-full overflow-y-auto rounded-lg border border-border bg-popover p-1 text-sm text-popover-foreground shadow-sm"
                        ></ul>
                    </div>
                }
                if overflow_searchable {
                    <div class="text-xs text-muted-foreground" data-options-hint="">
                        (overflow_hint)
                    </div>
                }
                ui_select(
                    attrs: attributes! {
                        id=(name.clone())
                        name=(name.clone())
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    },
                    for opt in option_views {
                        (opt)
                    }
                )
                if has_error {
                    ui_field_error(
                        attrs: attributes! {
                            id=(error_id.clone())
                            class="ac-error"
                            aria-live="polite"
                        },
                        (error_text)
                    )
                }
            )
        }
        .boxed())
    }
}

/// Typed multi-line text field bound to a Toasty field lens (GH #184).
///
/// `TextInput` renders `<input type="text">`, which is the wrong control for a
/// column holding prose — a post body, a description, a note. This is the same
/// field otherwise: one lens, the same lens-derived `required` default, the
/// same label and error contract, so the two are interchangeable in a `Schema`
/// and differ in the control (and in `TextInput`'s extra `email`/`unique`
/// modifiers, which have no textarea meaning).
///
/// A separate type rather than a `.multiline()` modifier on `TextInput`: the
/// control an author gets should be readable off the schema declaration, and
/// nothing in a Toasty `String` column says whether it holds prose.
#[derive(Debug, Clone)]
pub struct Textarea {
    name: String,
    label: String,
    required: bool,
    placeholder: Option<String>,
    rows: Option<u32>,
}

impl Textarea {
    /// Create a `Textarea` bound to the given field lens.
    ///
    /// Only `String` lenses compile, and `required` defaults from the field's
    /// nullability exactly as [`TextInput::r#for`] documents (GH #100).
    ///
    /// There is deliberately no `.unique()`: the app-side unique check builds
    /// its probe from `TextInput::eq_filter`, so a uniqueness modifier here
    /// would be a no-op that reads like a guarantee (GH #115 tracks the real
    /// fix). Declare uniqueness on a `TextInput` instead.
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let model = M::schema();
        let field = lens_field(path, &model);
        let label_str = lens_label(&field);
        Self {
            name: field.name.app_unwrap().to_string(),
            label: label_str,
            required: !field.nullable(),
            placeholder: None,
            rows: None,
        }
    }

    /// Create a `Textarea` bound to a lens inside an embedded struct or a
    /// `#[document]` (GH #185), resolving through the request's app schema so
    /// the leaf arrives as its flattened storage column. Same contract as
    /// [`TextInput::r#for_context`], including the not-required default.
    pub fn r#for_context<M>(cx: &Cx, path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let leaf = FieldResolver::from_cx(cx).resolve(path);
        Self {
            name: leaf.name,
            label: leaf.label,
            required: !leaf.nullable,
            placeholder: None,
            rows: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Opt out of the non-nullable default (GH #100), as [`TextInput::optional`].
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = Some(p.into());
        self
    }

    /// Fix the control's visible height in lines. Unset defers to the
    /// primitive, which grows with its content from a two-line minimum.
    pub fn rows(mut self, rows: u32) -> Self {
        self.rows = Some(rows);
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// Whether an empty submit fails validation (GH #88 semantics, as
    /// [`TextInput::is_required`]).
    pub fn is_required(&self) -> bool {
        self.required
    }

    pub fn validate(&self, value: &str) -> Vec<String> {
        let v = value.trim();
        let mut errs = Vec::new();
        if self.required && v.is_empty() {
            errs.push(format!("{} is required", self.label));
        }
        errs
    }

    /// Static render: the same `field` family chrome as [`TextInput`], with the
    /// stored value as the control's child (a `<textarea>` takes its initial
    /// value from content, not a `value` attribute).
    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        value: Option<&str>,
        errors: &[String],
        mode: Mode,
    ) -> Result<BoxView<'a>> {
        if mode == Mode::View {
            return render_value(cx, &self.label, value, ValueKind::Prose);
        }
        let label_text = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let placeholder = self.placeholder.clone();
        let rows = self.rows;
        let has_error = !errors.is_empty();
        let error_text = errors.first().cloned().unwrap_or_default();
        let value_owned = value.unwrap_or("").to_string();
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
                ui_textarea(
                    attrs: attributes! {
                        id=(name.clone())
                        name=(name.clone())
                        placeholder=(placeholder.clone())
                        rows=(rows)
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    },
                    (value_owned)
                )
                if has_error {
                    ui_field_error(
                        attrs: attributes! {
                            id=(error_id.clone())
                            class="ac-error"
                            aria-live="polite"
                        },
                        (error_text)
                    )
                }
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
///
/// On an edit, the stored path is surfaced as text and the control is left
/// **optional** (GH #184): a file input cannot be pre-filled, so a `required`
/// attribute on it made every edit blocking — the browser refuses to submit an
/// empty required file input, and the server's untouched-value backfill (which
/// exists for exactly this reason, see `panel/forms.rs`) never ran because the
/// request was never sent. `required` therefore keeps its create-time meaning
/// only, and an empty submit on an edit means "keep what is stored".
///
/// There is deliberately no `.required()`/`.optional()` control over that: the
/// two states are create and edit, which the field cannot know, so it is keyed
/// off whether a stored value was hydrated rather than off a declaration.
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
    /// upstream (#183).
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let field = lens_field(path, &M::schema());
        Self {
            name: field.name.app_unwrap().to_string(),
            label: lens_label(&field),
            required: !field.nullable(),
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
        value: Option<&str>,
        errors: &[String],
        mode: Mode,
    ) -> Result<BoxView<'a>> {
        // The detail page shows the stored path, never a file control
        // (GH #187): an empty `FileUpload` on an edit is the panel's "keep the
        // stored file" affordance, which is a statement about a form, not about
        // a record.
        if mode == Mode::View {
            return render_value(cx, &self.label, value, ValueKind::Machine);
        }
        let label_text = self.label.clone();
        let name = self.name.clone();
        // An edit hydrates the stored path; a create does not (GH #184). See
        // the type docs: the control is required only when nothing is stored,
        // since a file input cannot be pre-filled.
        let stored = value
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let is_edit = stored.is_some();
        let control_required = self.required && !is_edit;
        let has_error = !errors.is_empty();
        let error_text = errors.first().cloned().unwrap_or_default();
        let field_class = if has_error {
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        let hint_id = format!("{name}-hint");
        let described_by = match (has_error, is_edit) {
            (true, _) => Some(error_id.clone()),
            (false, true) => Some(hint_id.clone()),
            (false, false) => None,
        };
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
                    if control_required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                if let Some(current) = stored {
                    // The stored path is visible, so "there is no file" is no
                    // longer ambiguous, and the empty control reads as "leave
                    // it alone" rather than "this field is broken".
                    <div
                        class="text-xs text-muted-foreground"
                        data-file-current=(current.clone())
                    >
                        "Current: "
                        <span class="font-medium text-foreground">
                            (current.clone())
                        </span>
                    </div>
                }
                // The `input` primitive styles `type="file"` through its
                // `file:` classes and carries the `aria-invalid` error styling.
                ui_input(
                    attrs: attributes! {
                        id=(name.clone())
                        type="file"
                        name=(name.clone())
                        required=(control_required)
                        aria-required=(control_required.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(described_by)
                    }
                )
                if is_edit {
                    <div class="text-xs text-muted-foreground" id=(hint_id.clone())>
                        "Leave empty to keep the current file."
                    </div>
                }
                if has_error {
                    ui_field_error(
                        attrs: attributes! {
                            id=(error_id.clone())
                            class="ac-error"
                            aria-live="polite"
                        },
                        (error_text)
                    )
                }
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

    /// The opening tag that starts at `start`, sliced up to the `>` closing it.
    ///
    /// `Attributes` renders in no guaranteed order (topcoat#122), so a test
    /// locates a tag by whichever attribute it can and asserts on the whole
    /// tag. Quoting is honoured, so a `>` inside an attribute value (Tailwind
    /// selectors carry them) does not end the slice.
    fn opening_tag_at(html: &str, start: usize) -> &str {
        let mut quoted = false;
        for (offset, byte) in html.as_bytes()[start..].iter().enumerate() {
            match byte {
                b'"' => quoted = !quoted,
                b'>' if !quoted => return &html[start..start + offset],
                _ => {}
            }
        }
        panic!("unterminated tag at byte {start} in {html}");
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
        // No error → no error node: the primitive's contract is to render
        // `field_error` only when there is an error, so a valid field leaves
        // no empty `role="alert"` behind.
        assert!(
            !html.contains("text-sm text-destructive") && !html.contains("role=\"alert\""),
            "a valid field must not render an error slot in {html}"
        );
        // label derived from lens: DummyUser::fields().name() → "name" → "Name"
        assert!(html.contains(">Name"), "missing label in {html}");
    }

    #[tokio::test]
    async fn textarea_renders_a_multiline_control_with_the_stored_value() {
        // GH #184: prose columns get a `<textarea>`, not a one-line input. The
        // value is the control's child — a textarea has no `value` attribute.
        let cx = cx();
        let schema = Schema::new(Textarea::r#for(DummyUser::fields().name()).rows(4));
        let mut values = HashMap::new();
        values.insert("name".to_string(), "Line one\nLine two".to_string());
        let html = schema
            .render_with(&cx, &values, &HashMap::new())
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("<textarea"),
            "Textarea must render a textarea control, got {html}"
        );
        assert!(
            !html.contains("<input"),
            "a Textarea must not render an input, got {html}"
        );
        assert!(
            html.contains("Line one") && html.contains("Line two"),
            "the stored value must be the control's content, got {html}"
        );
        assert!(
            html.contains("rows=\"4\""),
            "declared rows must reach the control, got {html}"
        );
        assert!(
            html.contains(">Name"),
            "label must still derive from the lens, got {html}"
        );
        assert!(
            html.contains("data-slot=\"field\""),
            "the field family chrome must match TextInput, got {html}"
        );
    }

    #[tokio::test]
    async fn textarea_shares_the_required_contract_with_text_input() {
        // The two text fields differ in control only: presence validation and
        // the required marker follow the same lens-derived default.
        let schema = Schema::new(Textarea::r#for(DummyUser::fields().name()));
        let errors = schema.validate(&HashMap::new());
        assert_eq!(
            errors.get("name"),
            Some(&vec!["Name is required".to_string()]),
            "a non-nullable String column is required by default, as TextInput"
        );

        let optional = Schema::new(Textarea::r#for(DummyUser::fields().name()).optional());
        assert!(
            optional.validate(&HashMap::new()).is_empty(),
            "optional() must opt out of the required default"
        );

        // An omitted key is validated as empty, matching TextInput (GH #89).
        let whitespace = schema.validate(&HashMap::from([("name".to_string(), "   ".to_string())]));
        assert_eq!(
            whitespace.get("name"),
            Some(&vec!["Name is required".to_string()]),
            "whitespace-only counts as empty, matching the trim convention"
        );
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
        // A required-but-valid field renders no error node either.
        assert!(
            !html_req.contains("text-sm text-destructive"),
            "a valid required field must not render an error slot in {html_req}"
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
        // optional email: empty is ok, whitespace trimmed — on a field without
        // a unique constraint. `DummyUser.email` is `#[unique]`, so it is
        // required whatever else is declared (GH #189); `NullableRef.parent_id`
        // is the non-unique nullable column that pins the old behaviour.
        assert!(
            Select::r#for(NullableRef::fields().parent_id())
                .optional()
                .validate("")
                .is_empty(),
            "an optional, non-unique field must still accept empty (GH #100)"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().email())
                .email()
                .validate(" a@b.com ")
                .is_empty(),
            "email should trim"
        );
    }

    /// GH #189: the unique marker is presence, so `.optional()` cannot lift it
    /// — in the builder or from the lens. The email regex still applies to what
    /// is submitted.
    #[test]
    fn unique_implies_required_in_either_declaration_order() {
        let mut declarations = vec![
            TextInput::r#for(DummyUser::fields().email())
                .optional()
                .unique(),
            TextInput::r#for(DummyUser::fields().email())
                .unique()
                .optional(),
        ];
        // Derived from the lens, with no `.unique()` call at all: the rule
        // follows the column, not the declaration style.
        declarations.push(TextInput::r#for(DummyUser::fields().email()).optional());

        for (nth, input) in declarations.iter().enumerate() {
            assert!(
                input.is_unique() && input.is_required(),
                "declaration {nth} must be unique and required"
            );
            assert_eq!(
                input.validate(""),
                vec!["Email is required".to_string()],
                "declaration {nth}: an empty unique field is required, not absent"
            );
            assert_eq!(
                input.validate("   "),
                vec!["Email is required".to_string()],
                "declaration {nth}: whitespace-only counts as empty, as everywhere else"
            );
            assert!(
                input.validate("a@b.com").is_empty(),
                "declaration {nth}: a present value still validates normally"
            );
        }
    }

    /// GH #189: the marker a user sees reads the same predicate validation
    /// does, so a unique field cannot be refused for emptiness while rendering
    /// as optional — the disagreement that would have shipped had only
    /// `validate` been taught the rule.
    #[tokio::test]
    async fn unique_field_renders_the_required_marker() {
        let cx = cx();
        let html = Schema::new(
            TextInput::r#for(DummyUser::fields().email())
                .unique()
                .optional(),
        )
        .render(&cx)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
        assert!(
            html.contains("required") && html.contains("aria-required"),
            "a unique field is required in the markup too"
        );
        assert!(
            html.contains("text-destructive"),
            "the required asterisk must render"
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
        // widens upstream (#183).
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

    /// The opening `<input …>` tag around the file control, so assertions do
    /// not have to care about attribute order (topcoat#122).
    fn file_input_tag(html: &str) -> String {
        let at = html.find("type=\"file\"").expect("a file input");
        let start = html[..at].rfind("<input").expect("its opening tag");
        opening_tag_at(html, start).to_string()
    }

    fn cx_and_doc_schema() -> (Cx, Schema) {
        #[derive(Debug, toasty::Model)]
        struct Upload {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
        }
        (
            cx(),
            Schema::new(FileUpload::r#for(Upload::fields().path())),
        )
    }

    async fn render_upload(schema: &Schema, cx: &Cx, value: Option<&str>) -> String {
        let mut values = HashMap::new();
        if let Some(value) = value {
            values.insert("path".to_string(), value.to_string());
        }
        schema
            .render_with(cx, &values, &HashMap::new())
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(cx)
    }

    /// GH #184: nothing stored (a create) keeps the required contract — the
    /// browser blocks an empty submit and the server reports it inline.
    #[tokio::test]
    async fn file_upload_is_required_on_create() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_upload(&schema, &cx, None).await;
        let tag = file_input_tag(&html);
        assert!(
            tag.contains("required"),
            "a create must keep the required file control, got {tag}"
        );
        assert!(
            !html.contains("data-file-current"),
            "a create has no stored path to show, got {html}"
        );
        assert!(
            !html.contains("Leave empty"),
            "the keep-current hint is an edit affordance, got {html}"
        );
    }

    /// GH #184: a stored path (an edit) makes the control optional and shows
    /// what is stored, because a file input cannot be pre-filled — otherwise
    /// the browser blocks every save and the server's untouched-value backfill
    /// never gets a request to act on.
    #[tokio::test]
    async fn file_upload_surfaces_the_stored_path_and_drops_required() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_upload(&schema, &cx, Some("/uploads/cover.jpg")).await;
        let tag = file_input_tag(&html);
        assert!(
            !tag.contains("required"),
            "an edit must not block on the empty file control, got {tag}"
        );
        assert!(
            tag.contains("aria-describedby=\"path-hint\""),
            "the control must describe itself with the hint, got {tag}"
        );
        assert!(
            html.contains("data-file-current=\"/uploads/cover.jpg\""),
            "the stored path must be surfaced, got {html}"
        );
        assert!(
            html.contains("Leave empty to keep the current file."),
            "the edit must say an empty control keeps the file, got {html}"
        );
    }

    /// A stored path that is only whitespace is not a file: it must behave as
    /// a create, not as an edit with a blank "Current:" line.
    #[tokio::test]
    async fn file_upload_treats_a_blank_stored_path_as_empty() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_upload(&schema, &cx, Some("   ")).await;
        let tag = file_input_tag(&html);
        assert!(
            tag.contains("required"),
            "a blank stored path must stay required, got {tag}"
        );
        assert!(
            !html.contains("data-file-current"),
            "a blank stored path must not render a Current line, got {html}"
        );
    }

    #[tokio::test]
    async fn searchable_select_renders_filter_input() {
        // GH #91: opt-in client-side option search; default selects stay bare.
        let cx = CxTestBuilder::new().build();
        let plain = Select::r#for(DummyUser::fields().name()).options(vec!["a".to_string()]);
        let html = plain
            .render_with(&cx, None, &[], Mode::Form)
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
            .render_with(&cx, None, &[], Mode::Form)
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
        // GH #184: the filter is only useful with a list it can narrow. The
        // native popup is browser chrome the script cannot touch, so the
        // searchable markup carries its own listbox — rendered empty and
        // hidden, and filled by `selects.js`.
        assert!(
            html.contains("data-options-combobox") && html.contains("data-options-list"),
            "searchable select must render the suggestion list, got {html}"
        );
        assert!(
            html.contains("role=\"listbox\""),
            "the suggestion list must be a listbox, got {html}"
        );
        let list_at = html.find("data-options-list").expect("the list");
        let list_tag_start = html[..list_at].rfind("<ul").expect("its <ul>");
        let list_tag_end = html[list_tag_start..].find('>').expect("the tag's end");
        assert!(
            html[list_tag_start..list_tag_start + list_tag_end].contains("hidden"),
            "the list must render hidden until the field is used, got {html}"
        );
    }

    #[tokio::test]
    async fn select_renders_through_the_select_primitive() {
        // The schema select was a hand-rolled `<select>` on the old input
        // chrome (`rounded-md`, page fill, no focus ring); it now composes the
        // synced `select` primitive, so it matches the `input` beside it and
        // `selects.js` keeps finding the control inside the filterable field.
        let cx = CxTestBuilder::new().build();
        let schema =
            Schema::new(Select::r#for(DummyUser::fields().name()).options(vec!["a".to_string()]));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("has-[:disabled]:opacity-50") && html.contains("focus-visible:ring-2"),
            "select must compose the primitive's chrome, got {html}"
        );
        assert!(
            !html.contains("bg-background px-3 py-1"),
            "the hand-rolled select chrome must be gone, got {html}"
        );
        // `Attributes` renders in no guaranteed order (topcoat#122), so slice
        // the whole opening tag; quoting is honoured, so a `>` inside the
        // picker's Tailwind selectors does not end it early.
        let select_start = html.find("<select").expect("native select element");
        let tag = opening_tag_at(&html, select_start);
        assert!(
            tag.contains("name=\"name\"")
                && tag.contains("id=\"name\"")
                && tag.contains("aria-invalid=\"false\""),
            "attributes must reach the native control, got {tag}"
        );
    }
}
