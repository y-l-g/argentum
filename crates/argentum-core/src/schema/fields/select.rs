use argentum_ui::{
    field as ui_field, field_error as ui_field_error, field_label as ui_field_label,
    input as ui_input, select as ui_select,
};
use topcoat::{Result, context::Cx, view::*};

use super::{
    super::{
        lenses::{capitalize, lens_field, lens_label},
        relationship::{
            OptionLoadError, OptionSource, RelatedCheck, RelatedPrimaryKey,
            RelationshipCheckFuture, RelationshipChecker, RelationshipLoadFuture,
            RelationshipLoader, RelationshipSearchLoader, related_record_check, related_records,
            related_records_search,
        },
        tree::Mode,
        validation::Rules,
    },
    ValueKind, render_value,
};

/// Select field bound to a lens (often a foreign key like `author_id`).
///
/// `Select::for(Post::fields().author_id()).relationship(AuthorResource::query, |a| a.id, |a|
/// a.name.clone())` loads options through the related resource's tenant-scoped query
/// (GH #223) and stores the
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
    /// The embedded value whose variant this control chooses (GH #191): the
    /// discriminant column. When set, the control is a **variant driver** — it
    /// renders `data-variant-select`, and `variant.js` keeps only the groups
    /// marked with that column and the chosen value visible.
    variant_of: Option<String>,
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
            .field("variant_of", &self.variant_of)
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
            variant_of: self.variant_of.clone(),
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
            variant_of: None,
            options_static: Vec::new(),
            relationship: None,
            relationship_search: None,
            relationship_check: None,
        }
    }

    /// Create a `Select` over a column the schema generates no lens for
    /// (GH #191): the **discriminant** of an embedded enum.
    ///
    /// The one caller is [`discriminant_select`](crate::schema::discriminant_select),
    /// which fills the options from the app schema's variant list — each one
    /// submitting the variant's stored value and reading as its name. A
    /// discriminant column has no typed accessor, so there is no lens to bind
    /// and the control takes its name instead — exactly as the record fn reads
    /// it, out of the ordinary value map.
    ///
    /// It is never required: an empty submit is "no variant named", which the
    /// value codec answers with its own fallback (a create form has no stored
    /// variant to hydrate), so refusing it would make the fallback
    /// unreachable. It is also the **driver** of the derived variant groups:
    /// it renders `data-variant-select`, and `variant.js` shows only the group
    /// whose `data-variant-of` names this column and whose `data-variant` is
    /// the chosen value. A read-only page names the stored variant instead of
    /// the control (see [`Self::render_with`]).
    pub(crate) fn named(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            label: capitalize(&name),
            variant_of: Some(name.clone()),
            name,
            required: false,
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
    /// substring for bounded sets, and a pick writes the chosen option onto the
    /// select, which stays the form control. Past the cap it fetches
    /// `GET {parent_list_url}/options?field=&q=` (debounced, abort in-flight,
    /// selection preserved) and re-renders the list from the answer. Reuses the
    /// related `Table`'s declared `searchable()` columns; non-searchable
    /// selects keep the cap error. No-JS keeps the plain select.
    ///
    /// The list exists because the select cannot show filtering itself: the
    /// primitive opts into `appearance: base-select`, whose popup is browser
    /// chrome that ignores `option[hidden]`, so narrowing the select's own
    /// options is invisible (GH #184).
    ///
    /// Behavior asset: `assets/selects.js` (`argentum_ui::SELECTS_JS`, hooks
    /// `data-select-filterable` / `data-options-filter` /
    /// `data-options-combobox` / `data-options-list`), emitted by
    /// `Panel::render_document` on every document with shell assets
    /// (see ADR-0014). Without it the input is inert and the plain select
    /// keeps working.
    ///
    /// The input is the combobox: it carries `role="combobox"`,
    /// `aria-controls` naming the listbox, `aria-autocomplete="list"` and a
    /// collapsed `aria-expanded`, and `selects.js` keeps `aria-expanded` and
    /// `aria-activedescendant` in step with the popup. On an edit form the
    /// input starts on the current option's label, so the box over the hidden
    /// select shows what is stored.
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

    /// Load options via a related source's tenant-scoped query, a typed
    /// primary-key projection, and a label closure.
    ///
    /// `R` is any [`OptionSource`] (GH #208) — every `Resource` is one through
    /// the blanket impl in `resource`. The first argument is the related
    /// resource's `query` fn (e.g. `AuthorResource::query`) — it is only a
    /// type-inference witness; the loader calls the source's scoped query
    /// directly, so the tenant gate and the framework's derived tenant filter
    /// apply (GH #223). The second argument projects each related record to the
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
        R: OptionSource + 'static,
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
            }) as RelationshipLoadFuture
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
            // A misdeclaration (GH #223) is permanent: retrying cannot fix it,
            // so it is reported without the retry wording.
            Err(OptionLoadError::Misdeclared) => {
                vec![format!("{} could not load options", self.label)]
            }
        }
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// Validate a raw string value (required + empty). Existence is async via `validate_async`.
    pub fn validate(&self, value: &str) -> Vec<String> {
        Rules::new().validate(&self.label, self.required, value)
    }

    /// Async existence check: if relationship is configured and value non-empty, ensure it matches
    /// a loaded option.
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
                    // Permanent (GH #223): no retry wording, same as a denial.
                    Err(OptionLoadError::Misdeclared) => {
                        errs.push(format!("{} could not load options", self.label));
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
        // A variant driver (GH #191) reads as the variant's **name** on a
        // read-only page — `Published`, never the `3` the column holds, which
        // is a machine value a reader gets nothing from (ADR-0016). It is the
        // one row that says which state the record is in: the payload rows
        // beside it are all of every variant's, so on their own they do not.
        //
        // A record with no stored variant has no name to show, and neither has
        // one whose value the schema does not declare (data the control cannot
        // read back either): both render **nothing**, which is what the
        // pre-#191 hidden control did in view mode.
        if mode == Mode::View && self.variant_of.is_some() {
            let stored = value.unwrap_or("").trim();
            let named = self
                .options_static
                .iter()
                .find(|(v, _)| v == stored)
                .map(|(_, name)| name.clone());
            return match named {
                Some(name) => render_value(cx, &self.label, Some(&name), ValueKind::Prose),
                None => Ok(().boxed()),
            };
        }
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
            Err(OptionLoadError::LoadFailed)
                | Err(OptionLoadError::Overflow)
                | Err(OptionLoadError::Misdeclared)
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
        // the control uses the primitive's chrome.
        let field_class = if has_error {
            "ac-field ac-field--error"
        } else {
            "ac-field"
        };
        let error_id = format!("{name}-error");
        let list_id = format!("{name}-options-list");
        let filter_label = format!("Filter {label_text} options");
        // Server fetch only past the cap (GH #150): bounded searchable sets
        // keep the client-side label-substring filter (GH #91), so the
        // `data-options-server` flag must follow the overflow state — not
        // every searchable relationship. `selects.js` branches on this flag.
        let options_field = overflow_searchable.then(|| name.clone());
        let options_server = overflow_searchable.then_some("true");
        let options_overflow = overflow_searchable.then_some("true");
        let variant_of = self.variant_of.clone();
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
                    //
                    // The input and the list are one combobox (GH #293): the
                    // input carries the static ARIA (its role, the list it
                    // controls, list autocompletion), starts collapsed over the
                    // hidden list, and `selects.js` keeps `aria-expanded` and
                    // `aria-activedescendant` in step with the popup.
                    <div class="relative" data-options-combobox="">
                        ui_input(
                            attrs: attributes! {
                                type="search"
                                role="combobox"
                                aria-expanded="false"
                                aria-controls=(list_id.clone())
                                aria-autocomplete="list"
                                aria-label=(filter_label.clone())
                                placeholder="Filter…"
                                data-options-filter=""
                                class="h-9"
                                autocomplete="off"
                            }
                        )
                        <ul
                            id=(list_id.clone())
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
                        data-variant-select=(variant_of)
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

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::{
        super::test_support::{DummyUser, FkRef, attributes_of, opening_tag_at},
        *,
    };
    use crate::schema::Schema;

    /// A bare `Select` over a non-nullable FK rejects an empty submit inline
    /// (GH #147): an empty submit fails here with `is required`, so it never
    /// reaches the driver's `parse::<Uuid>("")`.
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
        // The `hidden` HTML boolean attribute, not a Tailwind class (`<ul>`'s
        // `class` carries none): the list must render hidden until the field is
        // used. Pinned as `hidden=""` so a class that merely contains the word
        // cannot satisfy it.
        assert!(
            html[list_tag_start..list_tag_start + list_tag_end].contains("hidden=\"\""),
            "the list must render hidden until the field is used, got {html}"
        );
        // GH #293: the input is the combobox, statically wired to the list it
        // filters. It starts collapsed over the hidden list, and names the
        // listbox; `selects.js` keeps `aria-expanded` and
        // `aria-activedescendant` in step with the popup.
        let filter_attrs = attributes_of(&html, "data-options-filter");
        for expected in [
            "role=\"combobox\"",
            "aria-expanded=\"false\"",
            "aria-controls=\"name-options-list\"",
            "aria-autocomplete=\"list\"",
        ] {
            assert!(
                filter_attrs.iter().any(|attr| attr == expected),
                "the filter input must carry {expected}, got {filter_attrs:?}"
            );
        }
        let list_attrs = attributes_of(&html, "data-options-list");
        assert!(
            list_attrs
                .iter()
                .any(|attr| attr == "id=\"name-options-list\""),
            "the listbox must carry the id the combobox controls, got {list_attrs:?}"
        );
    }

    #[tokio::test]
    async fn select_renders_through_the_select_primitive() {
        // The schema select composes the synced `select` primitive, so it
        // matches the `input` beside it and `selects.js` keeps finding the
        // control inside the filterable field.
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
        // GH #216: the primitive's *chrome* is paint; what it composes is
        // structural — the native `<select>` now sits inside the primitive's
        // wrapper `<span>`, which carries the checkmark style hook and the
        // chevron icon. The hand-rolled control was a bare `<select>`.
        let select_start = html.find("<select").expect("native select element");
        let wrapper_start = html[..select_start]
            .rfind("<span")
            .expect("the primitive's wrapper span");
        let wrapper_tag = opening_tag_at(&html, wrapper_start);
        assert!(
            wrapper_tag.contains("--select-checkmark"),
            "select must compose the select primitive's wrapper, got {wrapper_tag}"
        );
        assert!(
            html[select_start..].contains("<svg"),
            "the primitive's chevron must render, got {html}"
        );
        // `Attributes` renders in no guaranteed order (topcoat#122), so slice
        // the whole opening tag; quoting is honoured, so a `>` inside the
        // picker's Tailwind selectors does not end it early.
        let tag = opening_tag_at(&html, select_start);
        assert!(
            tag.contains("name=\"name\"")
                && tag.contains("id=\"name\"")
                && tag.contains("aria-invalid=\"false\""),
            "attributes must reach the native control, got {tag}"
        );
    }
}
