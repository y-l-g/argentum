//! `Resource` — maps one Toasty [`Model`] to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, pages, and navigation. See
//! `CONTEXT.md` and ADR-0002.

use std::marker::PhantomData;
use std::sync::Arc;

use std::collections::HashMap;

use argentum_ui::{
    ButtonSize, ButtonVariant, button, input as ui_input, pagination, pagination_content,
    pagination_item, pagination_next, pagination_previous, table, table_body, table_cell,
    table_head, table_header, table_row,
};
use toasty::stmt::{Expr, List, OrderByExpr};
use topcoat::context::Cx;
use topcoat::router::{Href, HrefParams, HrefQueries, HrefTarget};
use topcoat::{Result, view::*};

use crate::schema::{FieldLens, Schema, capitalize, lens_field_name_and_label};

/// Select filter — exact match on a `String` field (e.g. `status = "published"`).
pub struct SelectFilter<M> {
    name: String,
    label: String,
    lens: FieldLens<M, String>,
    options: Vec<String>,
}

impl<M> std::fmt::Debug for SelectFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("options", &self.options)
            .finish()
    }
}

impl<M> Clone for SelectFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            lens: self.lens.clone(),
            options: self.options.clone(),
        }
    }
}

impl<M> SelectFilter<M>
where
    M: toasty::schema::Model,
{
    pub fn new(lens: FieldLens<M, String>, options: Vec<String>) -> Self {
        let (name, label) = lens_field_name_and_label(lens.clone());
        Self {
            name,
            label,
            lens,
            options,
        }
    }

    /// Convenience so call sites read `SelectFilter::for(Post::fields().status(), vec![...])`.
    pub fn r#for(lens: FieldLens<M, String>, options: Vec<String>) -> Self {
        Self::new(lens, options)
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        // Only allow values in options; otherwise ignore (no filter).
        if !self.options.is_empty() && !self.options.contains(&v.to_string()) {
            return None;
        }
        Some(self.lens.clone().eq(v.to_string()))
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
    pub fn options(&self) -> &[String] {
        &self.options
    }
}

/// Ternary filter — `true` / `false` / `all` (no filter) on a `bool` field.
pub struct TernaryFilter<M> {
    name: String,
    label: String,
    lens: FieldLens<M, bool>,
}

impl<M> std::fmt::Debug for TernaryFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TernaryFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .finish()
    }
}

impl<M> Clone for TernaryFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            lens: self.lens.clone(),
        }
    }
}

impl<M> TernaryFilter<M>
where
    M: toasty::schema::Model,
{
    pub fn new(lens: FieldLens<M, bool>) -> Self {
        let (name, label) = lens_field_name_and_label(lens.clone());
        Self { name, label, lens }
    }

    pub fn r#for(lens: FieldLens<M, bool>) -> Self {
        Self::new(lens)
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        match value.trim() {
            "true" => Some(self.lens.clone().eq(true)),
            "false" => Some(self.lens.clone().eq(false)),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
}

/// Date filter — same-calendar-day match on a `Timestamp` field
/// (e.g. `created_at = "2024-01-15"` selects that whole day).
/// Range (`from`/`to`) support is future.
pub struct DateFilter<M> {
    name: String,
    label: String,
    lens: FieldLens<M, jiff::Timestamp>,
}

impl<M> std::fmt::Debug for DateFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DateFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .finish()
    }
}

impl<M> Clone for DateFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            lens: self.lens.clone(),
        }
    }
}

impl<M> DateFilter<M>
where
    M: toasty::schema::Model,
{
    pub fn new(lens: FieldLens<M, jiff::Timestamp>) -> Self {
        let (name, label) = lens_field_name_and_label(lens.clone());
        Self { name, label, lens }
    }

    pub fn r#for(lens: FieldLens<M, jiff::Timestamp>) -> Self {
        Self::new(lens)
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    /// Build the predicate for a submitted value (GH #93).
    ///
    /// Full RFC3339 timestamps match the exact instant (documented); a
    /// date-only `YYYY-MM-DD` matches the whole UTC day
    /// (`>= midnight AND < next midnight`), so rows stamped with any
    /// time-of-day still match.
    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        // Accept RFC3339 or YYYY-MM-DD (whole UTC day).
        if let Ok(ts) = v.parse::<jiff::Timestamp>() {
            return Some(self.lens.clone().eq(ts));
        }
        // Query decoding turns `+` into space, destroying numeric offsets
        // (`?filters=created_at:2024-01-15T09:30:00+02:00` arrives with a
        // space). A timestamp never legitimately contains a space, so retry
        // with `+` restored before giving up (GH #93).
        if v.contains(' ')
            && let Ok(ts) = v.replace(' ', "+").parse::<jiff::Timestamp>()
        {
            return Some(self.lens.clone().eq(ts));
        }
        if let Ok(date) = v.parse::<jiff::civil::Date>() {
            let start: jiff::Timestamp = format!("{date}T00:00:00Z").parse().ok()?;
            let end = start + jiff::Span::new().hours(24);
            return Some(self.lens.clone().ge(start).and(self.lens.clone().lt(end)));
        }
        None
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
}

/// Variant filter — exact match on an embedded-enum variant (e.g. `vehicule = "Moto"`).
///
/// Unlike [`SelectFilter`] (a `String` lens + options), a variant has no single
/// lens: Toasty stores it as one discriminant column plus one nullable column
/// per variant field. The caller therefore supplies prebuilt expressions —
/// typically `User::fields().vehicule().is_moto()` — one per option. Display
/// stays `TextColumn::computed` (see GH #77).
pub struct VariantFilter<M> {
    name: String,
    label: String,
    options: Vec<(String, Expr<bool>)>,
    _marker: std::marker::PhantomData<M>,
}

impl<M> std::fmt::Debug for VariantFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariantFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .field(
                "options",
                &self.options.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl<M> Clone for VariantFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            options: self.options.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M> VariantFilter<M>
where
    M: toasty::schema::Model,
{
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        options: Vec<(String, Expr<bool>)>,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            options,
            _marker: std::marker::PhantomData,
        }
    }

    /// Convenience alias so call sites read `VariantFilter::for("vehicule", "Véhicule", vec![...])`.
    pub fn r#for(
        name: impl Into<String>,
        label: impl Into<String>,
        options: Vec<(String, Expr<bool>)>,
    ) -> Self {
        Self::new(name, label, options)
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        self.options
            .iter()
            .find(|(k, _)| k == v)
            .map(|(_, e)| e.clone())
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
    pub fn options(&self) -> &[(String, Expr<bool>)] {
        &self.options
    }
}

/// Filter enum — the `Table::filters` seam.
#[derive(Debug, Clone)]
pub enum Filter<M> {
    Select(SelectFilter<M>),
    Ternary(TernaryFilter<M>),
    Date(DateFilter<M>),
    Variant(VariantFilter<M>),
}

impl<M> From<SelectFilter<M>> for Filter<M> {
    fn from(v: SelectFilter<M>) -> Self {
        Filter::Select(v)
    }
}
impl<M> From<TernaryFilter<M>> for Filter<M> {
    fn from(v: TernaryFilter<M>) -> Self {
        Filter::Ternary(v)
    }
}
impl<M> From<DateFilter<M>> for Filter<M> {
    fn from(v: DateFilter<M>) -> Self {
        Filter::Date(v)
    }
}
impl<M> From<VariantFilter<M>> for Filter<M> {
    fn from(v: VariantFilter<M>) -> Self {
        Filter::Variant(v)
    }
}

impl<M> Filter<M>
where
    M: toasty::schema::Model,
{
    pub fn name(&self) -> &str {
        match self {
            Filter::Select(f) => f.name(),
            Filter::Ternary(f) => f.name(),
            Filter::Date(f) => f.name(),
            Filter::Variant(f) => f.name(),
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Filter::Select(f) => f.label_str(),
            Filter::Ternary(f) => f.label_str(),
            Filter::Date(f) => f.label_str(),
            Filter::Variant(f) => f.label_str(),
        }
    }
    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        match self {
            Filter::Select(f) => f.to_expr(value),
            Filter::Ternary(f) => f.to_expr(value),
            Filter::Date(f) => f.to_expr(value),
            Filter::Variant(f) => f.to_expr(value),
        }
    }
}

/// Convert a single filter or tuple of filters into `Vec<Filter<M>>`.
pub trait IntoFilters<M> {
    fn into_filters(self) -> Vec<Filter<M>>;
}

impl<M> IntoFilters<M> for Filter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self]
    }
}
impl<M> IntoFilters<M> for SelectFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M> IntoFilters<M> for TernaryFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M> IntoFilters<M> for DateFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M> IntoFilters<M> for VariantFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M, A, B> IntoFilters<M> for (A, B)
where
    A: Into<Filter<M>>,
    B: Into<Filter<M>>,
{
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.0.into(), self.1.into()]
    }
}
impl<M, A, B, C> IntoFilters<M> for (A, B, C)
where
    A: Into<Filter<M>>,
    B: Into<Filter<M>>,
    C: Into<Filter<M>>,
{
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.0.into(), self.1.into(), self.2.into()]
    }
}

/// Text column bound to a typed lens **and** a typed projection.
///
/// The lens (`FieldLens<M, String>`) is the query side: it names the column
/// and produces search/sort predicates — `TextColumn::for(User::fields().name(), ..)`
/// fails to compile if the field does not exist (ADR-0001).
///
/// The projection closure is the render side: it reads the value off a model
/// instance for the cell (`|u| u.name.clone()`). Toasty models are plain
/// structs and expose no instance→field reflection, so the closure is the
/// only way to read a field generically (upstream gap #119: instance →
/// field-value extraction). A typo in the closure body fails at compile
/// time — there is no string dispatch and no panic at render.
#[derive(Clone)]
pub struct TextColumn<M> {
    /// The query-side lens; `None` for [`Self::computed`] columns, which
    /// render a value but declare no predicates.
    path: Option<FieldLens<M, String>>,
    name: String,
    label: String,
    project: Arc<dyn Fn(&M) -> String + Send + Sync>,
    searchable: bool,
    sortable: bool,
}

impl<M> TextColumn<M>
where
    M: toasty::schema::Model,
{
    /// Bind a column to a `String` field lens plus a projection closure.
    ///
    /// The closure receives each rendered row and returns the cell text, so
    /// computed cells (`|u| u.active.then(|| "Active".into()).unwrap_or_default()`)
    /// are as natural as plain field reads.
    pub fn for_lens(
        path: FieldLens<M, String>,
        project: impl Fn(&M) -> String + Send + Sync + 'static,
    ) -> Self {
        let (field_name, label) = lens_field_name_and_label(path.clone());
        Self {
            path: Some(path),
            name: field_name,
            label,
            project: Arc::new(project),
            searchable: false,
            sortable: false,
        }
    }

    /// A computed, display-only column (CONTEXT.md Column: "a computed value").
    ///
    /// No field lens — so it cannot be searchable or sortable (it maps to no
    /// query predicate) — but any cell projection compiles: booleans,
    /// timestamps, joined values. Calling `.searchable()` / `.sortable()` on
    /// a computed column panics (GH #101): a lying sort link / search promise
    /// is worse than a loud build error.
    pub fn computed(
        label: impl Into<String>,
        project: impl Fn(&M) -> String + Send + Sync + 'static,
    ) -> Self {
        let label = label.into();
        let name = label.to_lowercase();
        Self {
            path: None,
            name,
            label,
            project: Arc::new(project),
            searchable: false,
            sortable: false,
        }
    }

    /// Convenience alias so call sites read
    /// `TextColumn::for(User::fields().name(), |u| u.name.clone())`.
    pub fn r#for(
        path: FieldLens<M, String>,
        project: impl Fn(&M) -> String + Send + Sync + 'static,
    ) -> Self {
        Self::for_lens(path, project)
    }

    pub fn searchable(mut self) -> Self {
        assert!(
            self.path.is_some(),
            "searchable() on computed column '{}': computed columns map to no query predicate",
            self.label
        );
        self.searchable = true;
        self
    }

    pub fn sortable(mut self) -> Self {
        assert!(
            self.path.is_some(),
            "sortable() on computed column '{}': computed columns map to no query predicate",
            self.label
        );
        self.sortable = true;
        self
    }

    pub fn is_searchable(&self) -> bool {
        self.searchable
    }

    pub fn is_sortable(&self) -> bool {
        self.sortable
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// App-level field name (from the lens). Identifies the column in the
    /// `?sort=` URL parameter.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Render the cell for one row via the typed projection.
    pub fn render_cell(&self, row: &M) -> String {
        (self.project)(row)
    }

    pub fn to_search_expr(&self, term: &str) -> Option<Expr<bool>> {
        let t = term.trim();
        if !self.searchable || t.is_empty() {
            return None;
        }
        Some(self.path.clone()?.starts_with(t.to_string()))
    }

    pub fn to_order_by(&self, descending: bool) -> Option<OrderByExpr> {
        if self.sortable {
            // Cursor determinism is the engine's job: toasty's
            // `normalize_cursor_order` appends the physical PK columns to
            // ambiguous cursor orderings internally (GH #76).
            let path = self.path.clone()?;
            Some(if descending { path.desc() } else { path.asc() })
        } else {
            None
        }
    }
}

impl<M> std::fmt::Debug for TextColumn<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextColumn")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("searchable", &self.searchable)
            .field("sortable", &self.sortable)
            .finish_non_exhaustive()
    }
}

/// Column enum — Phase 1: only `Text`. Will generalize to `Number`, `Badge`, etc. later.
#[derive(Clone)]
pub enum Column<M> {
    Text(TextColumn<M>),
}

impl<M> From<TextColumn<M>> for Column<M> {
    fn from(v: TextColumn<M>) -> Self {
        Column::Text(v)
    }
}

impl<M> Column<M>
where
    M: toasty::schema::Model,
{
    pub fn label(&self) -> &str {
        match self {
            Column::Text(c) => c.label(),
        }
    }

    /// App-level field name (from the lens); identifies the column in URLs.
    pub fn name(&self) -> &str {
        match self {
            Column::Text(c) => c.name(),
        }
    }

    pub fn is_searchable(&self) -> bool {
        match self {
            Column::Text(c) => c.is_searchable(),
        }
    }

    pub fn is_sortable(&self) -> bool {
        match self {
            Column::Text(c) => c.is_sortable(),
        }
    }

    /// Render the cell for one row via the column's typed projection.
    pub fn render_cell(&self, row: &M) -> String {
        match self {
            Column::Text(c) => c.render_cell(row),
        }
    }

    pub fn to_search_expr(&self, term: &str) -> Option<Expr<bool>> {
        match self {
            Column::Text(c) => c.to_search_expr(term),
        }
    }

    pub fn to_order_by(&self, descending: bool) -> Option<OrderByExpr> {
        match self {
            Column::Text(c) => c.to_order_by(descending),
        }
    }
}

/// Convert a single column or tuple of columns into `Vec<Column<M>>`.
///
/// 5-tuple limit: without variadic generics this is idiomatic Rust — matches
/// `IntoSchema` in `schema.rs`. Tables wider than five columns are rare in
/// admin UIs; extend (or macro-ify) when a real Resource needs it.
pub trait IntoColumns<M> {
    fn into_columns(self) -> Vec<Column<M>>;
}

impl<M> IntoColumns<M> for TextColumn<M> {
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.into()]
    }
}

impl<M> IntoColumns<M> for Column<M> {
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self]
    }
}

impl<M, A, B> IntoColumns<M> for (A, B)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.0.into(), self.1.into()]
    }
}

impl<M, A, B, C> IntoColumns<M> for (A, B, C)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
    C: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.0.into(), self.1.into(), self.2.into()]
    }
}

impl<M, A, B, C, D> IntoColumns<M> for (A, B, C, D)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
    C: Into<Column<M>>,
    D: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.0.into(), self.1.into(), self.2.into(), self.3.into()]
    }
}

impl<M, A, B, C, D, E> IntoColumns<M> for (A, B, C, D, E)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
    C: Into<Column<M>>,
    D: Into<Column<M>>,
    E: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![
            self.0.into(),
            self.1.into(),
            self.2.into(),
            self.3.into(),
            self.4.into(),
        ]
    }
}

/// Row-key projection: reads the row identity off one model instance
/// (typically `|u| u.id.to_string()`). Toasty models are plain structs with
/// no instance→field reflection, so the key cannot be extracted generically
/// (upstream gap #119).
pub type RowKey<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

/// Table description of a `Resource`'s list view. Declares columns and how they map to queries.
///
/// Row identity is mandatory and typed: [`Table::id`] declares the row-key
/// projection and [`Table::render`] errors without it — the old stringly-typed
/// `HasId`/`GetField` dispatch (GH #10) is gone, cells render via
/// [`TextColumn`]'s lens-bound closure where typos fail at compile time
/// instead of panicking at render.
pub type GroupKey<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

/// A named grouping a `Table` can render: `name` is the `?group_by=` value
/// the table accepts, `key` projects a row to its group label (GH #92).
pub struct GroupDef<M> {
    name: String,
    key: GroupKey<M>,
}

pub struct Table<M> {
    columns: Vec<Column<M>>,
    filters: Vec<Filter<M>>,
    group_by: Option<GroupDef<M>>,
    row_key: Option<RowKey<M>>,
    page_size: Option<usize>,
    search_ui: Option<bool>,
    show_skeleton: bool,
    is_boundary: bool,
    defer_initial: bool,
    delete_prefix: Option<String>,
    bulk_delete: bool,
    live_search: bool,
    _marker: PhantomData<M>,
}

impl<M> std::fmt::Debug for Table<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("columns", &self.columns)
            .field("filters", &self.filters.len())
            .field("group_by", &self.group_by.is_some())
            .field("row_key", &self.row_key.is_some())
            .field("page_size", &self.page_size)
            .field("search_ui", &self.search_ui)
            .field("show_skeleton", &self.show_skeleton)
            .field("is_boundary", &self.is_boundary)
            .field("defer_initial", &self.defer_initial)
            .field("delete_prefix", &self.delete_prefix)
            .field("bulk_delete", &self.bulk_delete)
            .field("live_search", &self.live_search)
            .finish()
    }
}

impl<M> std::fmt::Debug for Column<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Column::Text(c) => std::fmt::Debug::fmt(c, f),
        }
    }
}

impl<M> Default for Table<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Table<M> {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            filters: Vec::new(),
            group_by: None,
            row_key: None,
            page_size: None,
            search_ui: None,
            show_skeleton: false,
            is_boundary: true,
            defer_initial: false,
            delete_prefix: None,
            bulk_delete: false,
            live_search: false,
            _marker: PhantomData,
        }
    }

    /// Create a table for the given model. `cx` is reserved for future tenancy/policy scoping.
    pub fn r#for(_cx: &Cx) -> Self {
        Self::new()
    }

    /// Declare the row-key projection (typically `|u| u.id.to_string()`).
    ///
    /// Required before [`Self::render`]: row identity is not optional
    /// (`CONTEXT.md` Table) — renders without it return an error rather than
    /// falling back to loop indices. The projection must be injective within
    /// a page (GH #96): duplicate keys corrupt keyed diffs and bulk selection,
    /// and are debug-asserted at render time.
    pub fn id(mut self, key: impl Fn(&M) -> String + Send + Sync + 'static) -> Self {
        self.row_key = Some(Arc::new(key));
        self
    }

    /// Declare columns. Accepts a single column or tuple of columns.
    pub fn columns(mut self, cols: impl IntoColumns<M>) -> Self {
        self.columns = cols.into_columns();
        self
    }

    /// Declare filters. Accepts a single filter or tuple of filters.
    pub fn filters(mut self, filters: impl IntoFilters<M>) -> Self {
        self.filters = filters.into_filters();
        self
    }

    /// Filter predicate for the current `TableState` — `AND` of active filter exprs.
    pub fn filter_expr(&self, state: &TableState) -> Option<Expr<bool>>
    where
        M: toasty::schema::Model,
    {
        let mut exprs = Vec::new();
        for f in &self.filters {
            if let Some(v) = state.filters.get(f.name())
                && let Some(e) = f.to_expr(v)
            {
                exprs.push(e);
            }
        }
        if exprs.is_empty() {
            None
        } else {
            // `Expr::and` chain: first and all.
            let mut iter = exprs.into_iter();
            let first = iter.next().unwrap();
            Some(iter.fold(first, |acc, e| acc.and(e)))
        }
    }

    /// Requested filters that produced no predicate (GH #93): `(key:value, reason)`
    /// where reason is `"unknown filter"` (no declared filter owns the key)
    /// or `"invalid value"` (the declared filter rejected the value).
    ///
    /// The list view renders these as a `role=alert` banner and keeps a 200;
    /// the export refuses the request with 400 instead of silently
    /// over-sharing an effectively-unfiltered CSV.
    pub fn unapplied_filters(&self, state: &TableState) -> Vec<(String, String)>
    where
        M: toasty::schema::Model,
    {
        let mut out = Vec::new();
        for (key, value) in &state.filters {
            match self.filters.iter().find(|f| f.name() == key) {
                None => out.push((format!("{key}:{value}"), "unknown filter".to_string())),
                Some(f) if f.to_expr(value).is_none() => {
                    out.push((format!("{key}:{value}"), "invalid value".to_string()))
                }
                Some(_) => {}
            }
        }
        out.sort();
        out
    }

    /// Group rows in-memory by a named key (count summarizer). No GROUP BY SQL.
    ///
    /// `name` declares the `?group_by=` value this table accepts
    /// (e.g. `"status"`); any other value renders no group headers and is
    /// dropped from pager/sort/filter links (GH #92) instead of silently
    /// grouping by the single declared key. Counts are page-local.
    pub fn group_by(
        mut self,
        name: impl Into<String>,
        key: impl Fn(&M) -> String + Send + Sync + 'static,
    ) -> Self {
        self.group_by = Some(GroupDef {
            name: name.into(),
            key: Arc::new(key),
        });
        self
    }

    /// The declared grouping iff `state.group_by` names it (GH #92).
    fn effective_group_key(&self, state: &TableState) -> Option<GroupKey<M>> {
        match (&self.group_by, &state.group_by) {
            (Some(def), Some(want)) if def.name == *want => Some(def.key.clone()),
            _ => None,
        }
    }

    /// The `?group_by=` value to echo in pager/sort/filter links: only the
    /// declared name, never an unknown value (GH #92).
    fn effective_group_name(&self, state: &TableState) -> Option<String> {
        match (&self.group_by, &state.group_by) {
            (Some(def), Some(want)) if def.name == *want => Some(def.name.clone()),
            _ => None,
        }
    }

    /// Enable real cursor pagination with the given page size.
    ///
    /// Loaders pair this with toasty's `.paginate(per_page)` (via
    /// [`TablePage::from_toasty_page`]); the render then shows Previous/Next
    /// links built from the executed page's cursors — never fake page
    /// numbers. Also implies a deterministic PK ordering when the table
    /// declares no sortable column (see [`Self::order_bys_for_state`]).
    ///
    /// A zero page size is a programmer error: it fails loudly at render/load
    /// time with a descriptive error (GH #96), never a bare panic.
    pub fn paginate(mut self, per_page: usize) -> Self {
        self.page_size = Some(per_page);
        self
    }

    /// Whether the page size was declared via [`Self::paginate`].
    pub fn page_size(&self) -> Option<usize> {
        self.page_size
    }

    /// Return the row-key for a record, if the table has one.
    pub fn key_for(&self, record: &M) -> Option<String> {
        self.row_key.as_ref().map(|f| f(record))
    }

    /// Force the search toolbar on or off.
    ///
    /// Defaults to showing the toolbar whenever at least one column is
    /// `searchable()` — the header indicators never promise a search the
    /// page does not have.
    pub fn search(mut self, enabled: bool) -> Self {
        self.search_ui = Some(enabled);
        self
    }

    /// Keystroke-live search via the `table_search` shard (GH #104).
    ///
    /// When enabled, the toolbar renders a signal-backed input that
    /// re-renders the grid on every keystroke (morphing in place, so focus
    /// and typing survive) instead of a GET submit. The `?q=` GET form stays
    /// inside `<noscript>` as the no-JS fallback. Opt-in per resource; the
    /// shard authorizes itself (`can_view_any` + tenancy via
    /// `Resource::query`) and every arg is validated like the GET path.
    /// Per-row `can_view` is not applied here, matching the list page:
    /// page-local row filtering would mislabel pagination, so row scoping
    /// belongs in `Resource::query` (GH #86).
    /// Note: Topcoat coalesces same-tick keystrokes and aborts in-flight
    /// reruns (latest wins) but does no time-based debounce.
    pub fn live_search(mut self, enabled: bool) -> Self {
        self.live_search = enabled;
        self
    }

    /// Whether this table is a `Boundary` (default `true`).
    /// When `true`, the rendered grid is wrapped in a `data-boundary` region
    /// so future `defer`+`boundary` diffing can swap only the grid.
    /// Use `boundary(false)` to opt-out.
    pub fn boundary(mut self, enabled: bool) -> Self {
        self.is_boundary = enabled;
        self
    }

    /// Defer the initial load, showing skeleton rows until the data arrives.
    /// When `true`, the table renders skeleton placeholders on first paint;
    /// the streamed list renders the swap through [`Self::without_skeleton`]
    /// so the loaded rows always arrive.
    pub fn defer(mut self, enabled: bool) -> Self {
        self.defer_initial = enabled;
        self.show_skeleton = enabled;
        self
    }

    /// Clear the eager-skeleton flag for the streamed swap payload (GH #98).
    ///
    /// The list page streams `skeleton` as the `suspense` fallback, then swaps
    /// in `table.render(page)`. If the declared table has `.defer(true)`, the
    /// swap would be a second skeleton; the streamed path renders through a
    /// copy with the flag cleared so rows always arrive.
    pub fn without_skeleton(mut self) -> Self {
        self.show_skeleton = false;
        self.defer_initial = false;
        self
    }

    /// Whether the table is a `Boundary`.
    pub fn is_boundary(&self) -> bool {
        self.is_boundary
    }

    /// Whether the table defers its initial load.
    pub fn is_defer(&self) -> bool {
        self.defer_initial
    }

    /// Enable row-level `Delete` action. When set, each row renders a
    /// `Delete` button that POSTs to `{prefix}/{id}/delete` with
    /// `requires_confirmation` semantics.
    pub fn with_delete(mut self, prefix: String) -> Self {
        self.delete_prefix = Some(prefix);
        self
    }

    /// Enable bulk selection with `BulkDelete` action.
    pub fn with_bulk_delete(mut self, enabled: bool) -> Self {
        self.bulk_delete = enabled;
        self
    }

    /// Whether the bulk checkbox column renders: bulk selection plus a delete
    /// prefix to post to (GH #74).
    fn bulk_enabled(&self) -> bool {
        self.bulk_delete && self.delete_prefix.is_some()
    }

    /// Global search predicate — OR across searchable columns (portable `starts_with`).
    pub fn search_expr(&self, term: &str) -> Option<Expr<bool>>
    where
        M: toasty::schema::Model,
    {
        let t = term.trim();
        if t.is_empty() {
            return None;
        }
        let mut exprs = self.columns.iter().filter_map(|c| c.to_search_expr(t));
        let first = exprs.next()?;
        Some(exprs.fold(first, |acc, e| acc.or(e)))
    }

    /// First sortable column's order_by. Cursor determinism needs no
    /// app-level tie-breaker (see [`Self::order_bys`]).
    pub fn order_by(&self, descending: bool) -> Option<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        self.columns.iter().find_map(|c| c.to_order_by(descending))
    }

    /// Ordered list for the query: the first sortable column's order.
    /// Returns empty if no sortable column is declared.
    ///
    /// No app-level PK tie-breaker is appended: toasty's engine appends the
    /// physical PK columns to ambiguous cursor orderings internally
    /// (`normalize_cursor_order`, tokio-rs/toasty#1142), so page contents are
    /// deterministic on SQL backends without Argentum's help (GH #76).
    pub fn order_bys(&self) -> Vec<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        self.order_by(false).into_iter().collect()
    }

    /// Order-bys over the model's primary key (asc, in declared order) —
    /// built through the public facade (`Model::path_field` + `Path::asc`),
    /// no `toasty_core` needed.
    ///
    /// Used only when a paginated table declares no sortable column at all:
    /// toasty's planner requires an `ORDER BY` for cursor pagination and its
    /// normalization only extends a non-empty ordering, so the PK order must
    /// be declared app-side in that one case.
    ///
    /// # Panics
    ///
    /// Never panics: a non-root model has no primary key, so this returns
    /// empty (and debug-asserts) instead of panicking per request (GH #96) —
    /// the engine then reports its descriptive "requires an ORDER BY" error
    /// at load.
    fn pk_order_bys() -> Vec<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        let app_model = M::schema();
        let Some(root) = app_model.as_root() else {
            debug_assert!(
                false,
                "pk_order_bys: {} is not a root model; deterministic pagination needs its primary key",
                std::any::type_name::<M>()
            );
            return Vec::new();
        };
        root.primary_key
            .fields
            .iter()
            .map(|fid| M::path_field::<toasty::stmt::Value>(fid.index).asc())
            .collect()
    }

    /// Resolve the full query ordering for a request.
    ///
    /// Single source of truth for loaders and render:
    /// 1. `?sort=<column>&dir=asc|desc` when `<column>` names a declared
    ///    sortable column — that column's direction (toasty appends the PK
    ///    tie-breakers internally, see [`Self::order_bys`]);
    /// 2. otherwise the declared default (first sortable column asc);
    /// 3. otherwise, when the table is paginated, the PK alone — cursor
    ///    pagination requires a deterministic order even with no sortable
    ///    column, and toasty only *extends* an existing non-empty ordering.
    ///
    /// Loaders that also need the search term parse the state once with
    /// [`TableState::from_cx`] and pass it here (see
    /// `crate::panel::Panel`'s generic resource list handler).
    pub fn order_bys_for_state(&self, state: &TableState) -> Vec<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        if let Some(sort) = &state.sort
            && let Some(col) = self
                .columns
                .iter()
                .find(|c| c.is_sortable() && c.name() == sort.column)
            && let Some(ord) = col.to_order_by(sort.descending)
        {
            return vec![ord];
        }
        let out = self.order_bys();
        if out.is_empty() && self.page_size.is_some() {
            return Self::pk_order_bys();
        }
        out
    }

    /// Render the table for the given loaded page.
    ///
    /// Real chrome, no fake affordances: the header renders sort **links**
    /// driving `?sort=`/`?dir=` and a search toolbar driving `?q=` (shown by
    /// default when any column is `searchable()`), rows are keyed by the
    /// projection declared via [`Self::id`] per `CONTEXT.md` and rendered via
    /// each column's typed projection, pagination shows Previous/Next links
    /// built from the executed page's **real** cursors (never invented page
    /// numbers), and the empty state reflects whether a search was active.
    ///
    /// Composes the synced `argentum-ui` primitives and Token classes
    /// (`bg-background`, `border-border`, `shadow-sm`, `text-muted-foreground`)
    /// — no raw colors, no `ac-*`.
    ///
    /// # Errors
    ///
    /// Errors when the table has no row key ([`Self::id`]) or no columns —
    /// row identity is not optional, and neither is something to show.
    pub async fn render<'a>(&self, cx: &'a Cx, page: TablePage<M>) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        let state = TableState::from_cx(cx);
        let path = topcoat::context::try_request_context::<http::request::Parts>(cx)
            .map(|parts| parts.uri.path().to_string())
            .unwrap_or_default();
        self.render_with_state(cx, page, &state, &path).await
    }

    /// Render with explicit list state and path instead of reading them from
    /// `cx` — the seam a live-search shard needs (GH #74): shard requests hit
    /// `POST /_topcoat/runtime/shards/...`, so `TableState::from_cx` would see
    /// the endpoint URI, not the list page's `?q=/filters/sort`. Callers pass
    /// the page's state (or shard args rebuilt via
    /// [`TableState::from_live_args`]) and the list URL explicitly.
    pub async fn render_with_state<'a>(
        &self,
        cx: &'a Cx,
        page: TablePage<M>,
        state: &TableState,
        path: &str,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        if self.page_size == Some(0) {
            return Err(std::io::Error::other(
                "Table::render: paginate requires per_page > 0 (GH #96)",
            )
            .into());
        }
        if self.columns.is_empty() {
            return Err(std::io::Error::other(
                "Table::render: no columns declared — declare columns via Table::columns(..)",
            )
            .into());
        }
        let Some(row_key) = &self.row_key else {
            return Err(std::io::Error::other(
                "Table::render: no row key declared — declare one via Table::id(|row| ..)",
            )
            .into());
        };
        let row_key = row_key.clone();
        let is_boundary = self.is_boundary;
        // Eager skeleton demo path (Table::defer(true)): same markup the
        // streamed path uses as its suspense fallback.
        if self.show_skeleton {
            return self.render_skeleton(cx).await;
        }
        let delete_prefix = self.delete_prefix.clone();
        let with_bulk = self.bulk_enabled();
        let head = self
            .render_thead(cx, state, path, delete_prefix.is_some(), with_bulk)
            .await?;
        let show_search = self.search_enabled();
        let search_bar = if show_search {
            Some(self.render_search_bar(cx, state, path).await?)
        } else {
            None
        };
        let show_filters = !self.filters.is_empty();
        let filter_bar = if show_filters {
            Some(self.render_filter_bar(cx, state, path).await?)
        } else {
            None
        };
        let bulk_bar_view: BoxView<'_> = if with_bulk {
            let bulk_action = format!("{}/bulk-delete", self.delete_prefix.clone().unwrap());
            let csrf = crate::csrf::current_token(cx);
            view! {
                cx =>
                <form
                    method="post"
                    action=(bulk_action)
                    class="flex gap-2 p-3 border-b border-border"
                    data-bulk-form=""
                >
                    <input type="hidden" name="csrf_token" value=(csrf)>
                    <input
                        name="ids"
                        placeholder="ids comma-separated"
                        aria-label="Bulk delete ids (or tick rows below)"
                        class="w-64 border border-border rounded px-2 py-1 text-sm"
                    >
                    <button
                        class="inline-flex items-center justify-center rounded-md bg-destructive px-4 py-2 text-sm text-destructive-foreground"
                        type="submit"
                    >
                        "Bulk Delete"
                    </button>
                </form>
            }
            .boxed()
        } else {
            view! { cx => <span></span> }.boxed()
        };
        let pager = self.render_pager(cx, state, path, &page).await?;
        let csrf_token = crate::csrf::current_token(cx);
        // Fail-visible filters (GH #93): requested filters that produced no
        // predicate render as a `role=alert` banner; the list keeps a 200
        // while the export refuses with 400 (see `resource_export`).
        let filter_warning: Option<BoxView<'_>> = {
            let unapplied = self.unapplied_filters(state);
            if unapplied.is_empty() {
                None
            } else {
                let detail = unapplied
                    .iter()
                    .map(|(pair, reason)| format!("{pair} ({reason})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let text = format!("Ignored filter(s): {detail} — showing unfiltered results.");
                let dir = state
                    .sort
                    .as_ref()
                    .map(|s| if s.descending { "desc" } else { "asc" });
                let group = self.effective_group_name(state);
                let clear = build_url(
                    path,
                    &[
                        ("q", state.search.as_deref()),
                        ("sort", state.sort.as_ref().map(|s| s.column.as_str())),
                        ("dir", dir),
                        ("group_by", group.as_deref()),
                    ],
                );
                Some(
                    view! {
                        cx =>
                        <div
                            class="border-b border-destructive/30 bg-muted px-4 py-2 text-sm"
                            role="alert"
                        >
                            (text)
                            " "
                            <a href=(clear) class="underline">"Clear filters"</a>
                        </div>
                    }
                    .boxed(),
                )
            }
        };
        // Precompute the row presentation so template bodies capture only
        // owned data — the lazy view outlives this call, so it must never
        // borrow `self` or `page`.
        let row_data: Vec<(String, Vec<String>)> = page
            .rows
            .iter()
            .map(|row| {
                let key = row_key(row);
                let cells: Vec<String> = self
                    .columns
                    .iter()
                    .map(|col| col.render_cell(row))
                    .collect();
                (key, cells)
            })
            .collect();
        // Row keys must be injective within a page (GH #96): duplicates corrupt
        // keyed diffs and bulk selection (two rows, one checkbox value).
        debug_assert!(
            {
                let mut seen = std::collections::HashSet::new();
                row_data.iter().all(|(k, _)| seen.insert(k.clone()))
            },
            "duplicate Table::id keys in one page: Table::id must be injective"
        );

        if page.rows.is_empty() {
            let empty_cell = self
                .render_empty_cell(cx, state, path, delete_prefix.is_some(), with_bulk)
                .await?;
            let inner = view! {
                cx =>
                <div
                    class="rounded-xl border border-border overflow-hidden"
                    data-table-root=""
                >
                    if show_search {
                        (search_bar.expect("search bar built when enabled"))
                    }
                    if show_filters {
                        (filter_bar.expect("filter bar built when enabled"))
                    }
                    (bulk_bar_view)
                    table(
                        (head)
                        (empty_cell)
                    )
                </div>
            };
            return Ok(if is_boundary {
                view! { cx => <div data-boundary="table">(inner)</div> }.boxed()
            } else {
                inner.boxed()
            });
        }

        // Grouping (in-memory, count summarizer) — only when `?group_by=`
        // names the declared group; unknown values render nothing (GH #92).
        // Rendered after skeleton/empty so defer shows skeleton and empty shows
        // the honest empty state even when `?group_by=` is set (GH #75).
        // Counts are page-local (GH #92): label them as such so page 1 never
        // reads as a table total.
        let mut group_views: Vec<BoxView<'_>> = Vec::new();
        if let Some(group_fn) = self.effective_group_key(state) {
            use std::collections::BTreeMap;
            let mut groups: BTreeMap<String, usize> = BTreeMap::new();
            for row in &page.rows {
                *groups.entry(group_fn(row)).or_insert(0) += 1;
            }
            for (key, count) in groups {
                let text = format!("{} ({} on this page)", key, count);
                group_views.push(
                    view! {
                        cx =>
                        <div class="px-4 py-2 bg-muted text-sm font-medium">(text)</div>
                    }
                    .boxed(),
                );
            }
        }

        // One grid body for grouped and ungrouped pages: `group_views` is
        // empty unless `?group_by=` named the declared group.
        let inner = view! {
            cx =>
            <div
                class="rounded-xl border border-border overflow-hidden"
                data-table-root=""
            >
                if show_search {
                    (search_bar.expect("search bar built when enabled"))
                }
                if show_filters {
                    (filter_bar.expect("filter bar built when enabled"))
                }
                (bulk_bar_view)
                if let Some(warning) = filter_warning {
                    (warning)
                }
                for gv in group_views {
                    (gv)
                }
                table(
                    (head)
                    table_body(
                        for (key, cells) in &row_data {
                            let key_for_row = key.clone();
                            let key_for_action = key.clone();
                            let key_for_select = key.clone();
                            let csrf_for_row = csrf_token.clone();
                            let row_dom_id = row_dom_id(&key_for_row);
                            table_row(
                                key: key_for_row,
                                attrs: attributes! { id=(row_dom_id) },
                                if with_bulk {
                                    table_cell(
                                        <input
                                            type="checkbox"
                                            value=(key_for_select)
                                            aria-label="Select row"
                                            data-row-select=""
                                        >
                                    )
                                }
                                for cell in cells {
                                    table_cell((cell.clone()))
                                }
                                if let Some(prefix) = &delete_prefix {
                                    table_cell(
                                        <form
                                            method="post"
                                            action=(format!(
                                                "{}/{}/delete",
                                                prefix,
                                                encode_path_segment(&key_for_action),
                                            ))
                                        >
                                            <input
                                                type="hidden"
                                                name="csrf_token"
                                                value=(csrf_for_row)
                                            >
                                            button(
                                                variant: ButtonVariant::Ghost,
                                                size: ButtonSize::Md,
                                                attrs: attributes! { r#type="submit" },
                                                "Delete"
                                            )
                                        </form>
                                    )
                                }
                            )
                        }
                    )
                )
                for p in pager {
                    (p)
                }
            </div>
        };
        Ok(if is_boundary {
            view! { cx => <div data-boundary="table">(inner)</div> }.boxed()
        } else {
            inner.boxed()
        })
    }

    /// The skeleton placeholder grid — three pulsing rows under the real
    /// column header. This is the [`suspense`] fallback for tables whose rows
    /// stream in ([`Table::render`] also uses it for the eager
    /// `defer(true)` demo path). Wrapped in the same `data-boundary` region
    /// as the real grid so the markup shape matches when the swap arrives.
    /// Carries `aria-busy` while loading plus toolbar/pager pulse placeholders
    /// (GH #98) so the streamed chrome lands without a layout shift.
    pub async fn render_skeleton<'a>(&self, cx: &'a Cx) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let state = TableState::from_cx(cx);
        let path = topcoat::context::try_request_context::<http::request::Parts>(cx)
            .map(|parts| parts.uri.path().to_string())
            .unwrap_or_default();
        let head = self
            .render_thead(
                cx,
                &state,
                &path,
                self.delete_prefix.is_some(),
                self.bulk_enabled(),
            )
            .await?;
        let column_count = self.columns.len();
        let with_delete = self.delete_prefix.is_some();
        let with_bulk = self.bulk_enabled();
        let inner = view! {
            cx =>
            <div
                class="rounded-xl border border-border overflow-hidden"
                data-table-root=""
                aria-busy="true"
            >
                <div class="border-b border-border p-3" aria-hidden="true">
                    <div class="animate-pulse rounded-md bg-foreground/10 h-9 w-64"></div>
                </div>
                table(
                    (head)
                    table_body(
                        for i in 0..3 {
                            table_row(
                                key: i,
                                if with_bulk {
                                    table_cell(
                                        <div
                                            class="animate-pulse rounded-md bg-foreground/10 h-4 w-4"
                                        ></div>
                                    )
                                }
                                for _ in 0..column_count {
                                    table_cell(
                                        <div
                                            class="animate-pulse rounded-md bg-foreground/10 h-4 w-full"
                                        ></div>
                                    )
                                }
                                if with_delete {
                                    table_cell(
                                        <div
                                            class="animate-pulse rounded-md bg-foreground/10 h-4 w-12"
                                        ></div>
                                    )
                                }
                            )
                        }
                    )
                )
                <div class="border-t border-border p-3" aria-hidden="true">
                    <div class="animate-pulse rounded-md bg-foreground/10 h-9 w-40"></div>
                </div>
            </div>
        };
        Ok(if self.is_boundary {
            view! { cx => <div data-boundary="table">(inner)</div> }.boxed()
        } else {
            inner.boxed()
        })
    }

    /// Generate CSV for the given page (header + rows, RFC4180 escaped).
    ///
    /// Formula cells are defused per OWASP (a leading `'` is prepended when
    /// the cell starts with `=`, `+`, `-`, `@`, `|`, or `%`) so a stored
    /// value like `=1+1` opens as text, not a live spreadsheet formula.
    /// The page passed in is buffered as one `String`; the export handler
    /// caps the filtered query (GH #94) so callers cannot buffer an
    /// unbounded table.
    pub fn to_csv(&self, page: &TablePage<M>) -> String
    where
        M: toasty::schema::Model,
    {
        fn defuse_formula(s: &str) -> String {
            let trimmed = s.trim_start_matches([' ', '\t']);
            if let Some(first) = trimmed.chars().next()
                && matches!(first, '=' | '+' | '-' | '@' | '|' | '%')
            {
                return format!("'{s}");
            }
            s.to_string()
        }
        fn escape_csv(s: &str) -> String {
            let s = defuse_formula(s);
            if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s
            }
        }
        let mut out = String::new();
        let headers: Vec<String> = self.columns.iter().map(|c| escape_csv(c.label())).collect();
        out.push_str(&headers.join(","));
        out.push('\n');
        for row in &page.rows {
            let cells: Vec<String> = self
                .columns
                .iter()
                .map(|c| escape_csv(&c.render_cell(row)))
                .collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        out
    }

    /// Whether the search toolbar renders: the explicit `search(bool)` value,
    /// or auto — at least one `searchable()` column.
    fn search_enabled(&self) -> bool
    where
        M: toasty::schema::Model,
    {
        self.search_ui
            .unwrap_or_else(|| self.columns.iter().any(|c| c.is_searchable()))
    }

    /// The search toolbar (GET form); live tables instead render the host
    /// input eagerly and the shard invocation in the streamed region (see
    /// [`Self::render_live_search_bar`] / [`Self::render_live_invocation`]).
    async fn render_search_bar<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
    ) -> Result<BoxView<'a>> {
        let action = path.to_string();
        let q_display = state.search.clone().unwrap_or_default();
        let sort_hidden = state.sort.as_ref().map(|s| s.column.clone());
        let dir_hidden = state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" });
        let filters_hidden = state.filters_param();
        // Echo only the declared group name (GH #92): unknown `?group_by=`
        // values are dropped from links instead of round-tripping.
        let group_hidden = self.effective_group_name(state);
        let clear_url = state
            .sort
            .as_ref()
            .map(|s| {
                build_url(
                    path,
                    &[
                        ("sort", Some(s.column.as_str())),
                        ("dir", Some(if s.descending { "desc" } else { "asc" })),
                        ("filters", filters_hidden.as_deref()),
                        ("group_by", group_hidden.as_deref()),
                    ],
                )
            })
            .or_else(|| {
                filters_hidden.as_ref().map(|f| {
                    build_url(
                        path,
                        &[
                            ("filters", Some(f.as_str())),
                            ("group_by", group_hidden.as_deref()),
                        ],
                    )
                })
            })
            .or_else(|| {
                group_hidden
                    .as_deref()
                    .map(|g| build_url(path, &[("group_by", Some(g))]))
            });
        Ok(view! {
            cx =>
            <form
                method="get"
                action=(action)
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
            >
                if let Some(sort) = sort_hidden {
                    <input type="hidden" name="sort" value=(sort)>
                }
                if let Some(dir) = dir_hidden {
                    <input type="hidden" name="dir" value=(dir)>
                }
                if let Some(filters) = filters_hidden.clone() {
                    <input type="hidden" name="filters" value=(filters)>
                }
                if let Some(group_by) = group_hidden {
                    <input type="hidden" name="group_by" value=(group_by)>
                }
                ui_input(
                    attrs: attributes! {
                        type="search"
                        name="q"
                        value=(q_display)
                        placeholder="Prefix search…"
                        aria-label="Prefix search table"
                        class="w-64"
                    }
                )
                button(
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Md,
                    attrs: attributes! { type="submit" },
                    "Search"
                )
                if let Some(url) = clear_url {
                    <a
                        href=(url)
                        class="text-sm text-muted-foreground hover:text-foreground"
                    >
                        "Clear"
                    </a>
                }
            </form>
        }
        .boxed())
    }

    /// Whether this table renders the keystroke-live search host (GH #104).
    pub(crate) fn is_live_search(&self) -> bool {
        self.live_search
    }

    /// Eager live-search input for live tables (GH #104): the signal-backed
    /// input plus the GET form as `<noscript>` fallback. Rendered eagerly
    /// above the streamed region; the shard invocation that fills the grid
    /// lives in the streamed region ([`Self::render_live_invocation`]) so the
    /// grid can only ever render once per response.
    pub(crate) async fn render_live_search_bar<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        q: topcoat::runtime::Signal<String>,
    ) -> Result<BoxView<'a>> {
        use topcoat::runtime::Event;

        let fallback = self.render_search_bar(cx, state, path).await?;
        Ok(view! {
            cx =>
            <div
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
                data-live-search=""
            >
                <input
                    :value=$(q.get())
                    @input=$(|e: Event| q.set(e.target.value))
                    type="search"
                    placeholder="Prefix search…"
                    aria-label="Live prefix search table"
                    class="w-64"
                >
                <noscript>(fallback)</noscript>
            </div>
        }
        .boxed())
    }

    /// The `table_search` shard invocation filling a live table's streamed
    /// region (GH #104). Static snapshots (path, filters, sort, cursors)
    /// travel as constants; only `q` re-renders. Unchanged queries keep the
    /// page cursor so the inline output matches the current page; the first
    /// keystroke starts a fresh result set.
    pub(crate) async fn render_live_invocation<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        q: topcoat::runtime::Signal<String>,
    ) -> Result<BoxView<'a>> {
        use crate::panel::table_search;

        let live_path = path.to_string();
        let initial_q = state.search.clone().unwrap_or_default();
        let live_after = state.after.clone().unwrap_or_default();
        let live_before = state.before.clone().unwrap_or_default();
        let live_filters = state.filters_param().unwrap_or_default();
        let live_sort = state
            .sort
            .as_ref()
            .map(|s| s.column.clone())
            .unwrap_or_default();
        let live_dir = state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" })
            .unwrap_or("asc")
            .to_string();
        let live_group = self.effective_group_name(state).unwrap_or_default();
        // Snapshots travel as one encoded bundle (GH #104): it keeps the
        // shard arity small and lets future state fields ride free.
        let live_rest = encode_live_rest(
            &initial_q,
            &live_filters,
            &live_sort,
            &live_dir,
            &live_group,
        );
        Ok(view! {
            cx =>
            table_search(
                path: $(live_path.clone()),
                q: $(q.get()),
                after: $(live_after.clone()),
                before: $(live_before.clone()),
                rest: $(live_rest.clone())
            )
        }
        .boxed())
    }

    async fn render_filter_bar<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        if self.filters.is_empty() {
            return Ok(view! { cx => <span></span> }.boxed());
        }
        let action = path.to_string();
        let filters_display = state.filters_param().unwrap_or_default();
        let sort_hidden = state.sort.as_ref().map(|s| s.column.clone());
        let dir_hidden = state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" });
        let q_hidden = state.search.clone();
        let group_hidden = self.effective_group_name(state);
        let clear_url = if !state.filters.is_empty() {
            Some(build_url(
                path,
                &[
                    ("q", state.search.as_deref()),
                    ("sort", state.sort.as_ref().map(|s| s.column.as_str())),
                    ("dir", dir_hidden),
                    ("group_by", group_hidden.as_deref()),
                ],
            ))
        } else {
            None
        };
        // One typed control per declared filter (GH #74). Controls carry only
        // `data-filter-name` (no `name`, so they never submit on their own);
        // `filters.js` composes them into the single `filters` text field on
        // submit, which stays as the no-JS free-text fallback.
        let mut controls: Vec<BoxView<'_>> = Vec::with_capacity(self.filters.len());
        for f in &self.filters {
            let current = state.filters.get(f.name()).cloned().unwrap_or_default();
            match f {
                Filter::Select(s) => {
                    let name = s.name().to_string();
                    let label = s.label_str().to_string();
                    let aria = label.clone();
                    let mut opts = vec![String::new()];
                    opts.extend(s.options().iter().cloned());
                    let current_c = current.clone();
                    controls.push(
                        view! {
                            cx =>
                            <label
                                class="flex items-center gap-2 text-sm text-muted-foreground"
                            >
                                (label)
                                <select
                                    data-filter-name=(name)
                                    aria-label=(aria)
                                    class="flex h-9 rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
                                >
                                    for opt in opts {
                                        if opt.is_empty() {
                                            <option value="" selected=(current_c.is_empty())>
                                                "All"
                                            </option>
                                        } else {
                                            <option value=(opt.clone()) selected=(current_c == opt)>
                                                (opt)
                                            </option>
                                        }
                                    }
                                </select>
                            </label>
                        }
                        .boxed(),
                    );
                }
                Filter::Ternary(t) => {
                    let name = t.name().to_string();
                    let label = t.label_str().to_string();
                    let aria = label.clone();
                    let current_c = current.clone();
                    controls.push(
                        view! {
                            cx =>
                            <label
                                class="flex items-center gap-2 text-sm text-muted-foreground"
                            >
                                (label)
                                <select
                                    data-filter-name=(name)
                                    aria-label=(aria)
                                    class="flex h-9 rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
                                >
                                    <option value="" selected=(current_c.is_empty())>
                                        "All"
                                    </option>
                                    <option value="true" selected=(current_c == "true")>
                                        "True"
                                    </option>
                                    <option value="false" selected=(current_c == "false")>
                                        "False"
                                    </option>
                                </select>
                            </label>
                        }
                        .boxed(),
                    );
                }
                Filter::Date(d) => {
                    let name = d.name().to_string();
                    let label = d.label_str().to_string();
                    let aria = label.clone();
                    // `<input type=date>` needs YYYY-MM-DD; truncate RFC3339.
                    let date_value = current.split('T').next().unwrap_or(&current).to_string();
                    controls.push(
                        view! {
                            cx =>
                            <label
                                class="flex items-center gap-2 text-sm text-muted-foreground"
                            >
                                (label)
                                <input
                                    type="date"
                                    data-filter-name=(name)
                                    value=(date_value)
                                    aria-label=(aria)
                                    class="flex h-9 rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
                                >
                            </label>
                        }
                        .boxed(),
                    );
                }
                Filter::Variant(v) => {
                    let name = v.name().to_string();
                    let label = v.label_str().to_string();
                    let aria = label.clone();
                    let keys: Vec<String> = v.options().iter().map(|(k, _)| k.clone()).collect();
                    let current_c = current.clone();
                    controls.push(
                        view! {
                            cx =>
                            <label
                                class="flex items-center gap-2 text-sm text-muted-foreground"
                            >
                                (label)
                                <select
                                    data-filter-name=(name)
                                    aria-label=(aria)
                                    class="flex h-9 rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
                                >
                                    <option value="" selected=(current_c.is_empty())>
                                        "All"
                                    </option>
                                    for opt in keys {
                                        <option value=(opt.clone()) selected=(current_c == opt)>
                                            (opt)
                                        </option>
                                    }
                                </select>
                            </label>
                        }
                        .boxed(),
                    );
                }
            }
        }
        Ok(view! {
            cx =>
            <form
                method="get"
                action=(action)
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
                data-filters-form=""
            >
                if let Some(q) = q_hidden {
                    <input type="hidden" name="q" value=(q)>
                }
                if let Some(sort) = sort_hidden {
                    <input type="hidden" name="sort" value=(sort)>
                }
                if let Some(dir) = dir_hidden {
                    <input type="hidden" name="dir" value=(dir)>
                }
                if let Some(group_by) = group_hidden {
                    <input type="hidden" name="group_by" value=(group_by)>
                }
                for ctl in controls {
                    (ctl)
                }
                ui_input(
                    attrs: attributes! {
                        type="text"
                        name="filters"
                        value=(filters_display)
                        placeholder="filters e.g. status:published"
                        aria-label="Filter table (free text)"
                        class="w-64"
                    }
                )
                button(
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Md,
                    attrs: attributes! { type="submit" },
                    "Apply filters"
                )
                if let Some(url) = clear_url {
                    <a
                        href=(url)
                        class="text-sm text-muted-foreground hover:text-foreground"
                    >
                        "Clear filters"
                    </a>
                }
            </form>
        }
        .boxed())
    }

    /// The zero-rows cell — one honest message, not two: "no records yet"
    /// when unfiltered, "no results" with a Clear link when a search is
    /// active. The dead Create button is gone (create pages are not wired
    /// yet). Wrapped in a single cell spanning the table so it sits inside
    /// the grid.
    async fn render_empty_cell<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        with_delete: bool,
        with_bulk: bool,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let mut colspan = self.columns.len();
        if with_bulk {
            colspan += 1;
        }
        if with_delete {
            colspan += 1;
        }
        let filtered = state.search.is_some() || !state.filters.is_empty();
        let clear_url = if filtered {
            Some(match &state.sort {
                Some(s) => build_url(
                    path,
                    &[
                        ("sort", Some(s.column.as_str())),
                        ("dir", Some(if s.descending { "desc" } else { "asc" })),
                    ],
                ),
                None => path.to_string(),
            })
        } else {
            None
        };
        // Search is prefix-only (`starts_with`, GH #101): the empty copy says
        // so instead of implying general search.
        let message = match &state.search {
            Some(term) => format!("No prefix matches for \u{201c}{term}\u{201d}"),
            None if !state.filters.is_empty() => "No results for these filters".to_string(),
            None => "No records yet".to_string(),
        };
        let clear_label = if state.search.is_some() {
            "Clear search"
        } else {
            "Clear filters"
        };
        // Void window (GH #98): a cursor that lands past the last row (e.g.
        // rows deleted under pagination) leaves an empty page with no pager —
        // link back to the first page instead of a dead end. State is
        // preserved, only the cursor is dropped.
        let first_page_url = (state.after.is_some() || state.before.is_some()).then(|| {
            let dir = state
                .sort
                .as_ref()
                .map(|s| if s.descending { "desc" } else { "asc" });
            let filters = state.filters_param();
            let group = self.effective_group_name(state);
            build_url(
                path,
                &[
                    ("q", state.search.as_deref()),
                    ("sort", state.sort.as_ref().map(|s| s.column.as_str())),
                    ("dir", dir),
                    ("filters", filters.as_deref()),
                    ("group_by", group.as_deref()),
                ],
            )
        });
        Ok(view! {
            cx =>
            table_body(
                table_row(
                    key: "empty",
                    table_cell(
                        attrs: attributes! { colspan=(colspan) class="px-6 py-16 text-center" },
                        <div class="flex flex-col items-center gap-4">
                            <p class="text-sm text-muted-foreground">(message)</p>
                            if let Some(url) = clear_url {
                                <a href=(url) class="text-sm text-primary hover:underline">
                                    (clear_label)
                                </a>
                            }
                            if let Some(url) = first_page_url {
                                <a href=(url) class="text-sm text-primary hover:underline">
                                    "Back to first page"
                                </a>
                            }
                        </div>
                    )
                )
            )
        }
        .boxed())
    }

    /// Previous/Next pagination links from the executed page's real cursors.
    /// Empty when the table is not paginated or the page has no neighbors —
    /// no invented page numbers. Links preserve the search and sort state;
    /// cursors travel via `?after=`/`?before=`.
    async fn render_pager<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        page: &TablePage<M>,
    ) -> Result<Vec<BoxView<'a>>> {
        if self.page_size.is_none() {
            return Ok(Vec::new());
        }
        // Cursors only carry ordering values; the loader re-applies search and
        // sort, so the links must carry that state along.
        let dir = state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" });
        let filters_param = state.filters_param();
        let group_name = self.effective_group_name(state);
        let preserve: Vec<(&str, Option<&str>)> = vec![
            ("q", state.search.as_deref()),
            ("sort", state.sort.as_ref().map(|s| s.column.as_str())),
            ("dir", dir),
            ("filters", filters_param.as_deref()),
            ("group_by", group_name.as_deref()),
        ];
        let href = |param: &str, cursor: &str| {
            let mut params = Vec::with_capacity(preserve.len() + 1);
            params.push((param, Some(cursor)));
            params.extend(preserve.clone());
            build_url(path, &params)
        };
        let next_href = page
            .next_cursor
            .as_ref()
            .map(|cursor| href("after", cursor));
        let prev_href = page
            .prev_cursor
            .as_ref()
            .map(|cursor| href("before", cursor));
        if prev_href.is_none() && next_href.is_none() {
            return Ok(Vec::new());
        }
        let pager = view! {
            cx =>
            <div class="border-t border-border p-3">
                pagination(
                    pagination_content(
                        if let Some(href) = prev_href {
                            pagination_item(
                                pagination_previous(attrs: attributes! { href=(href) })
                            )
                        }
                        if let Some(href) = next_href {
                            pagination_item(
                                pagination_next(attrs: attributes! { href=(href) })
                            )
                        }
                    )
                )
            </div>
        };
        Ok(vec![pager.boxed()])
    }

    /// The shared column-header row — the single source of the `<thead>`
    /// markup: labels, `⌕` on searchable columns, and **links** on sortable
    /// columns that toggle `?sort=`/`?dir=` (↑/↓ with `aria-sort` when active,
    /// ↕ when inactive). Every render branch (skeleton / empty / rows)
    /// composes it, so an a11y or styling change happens once.
    async fn render_thead<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        with_delete: bool,
        with_bulk: bool,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        // The active sort only counts when it names a declared sortable column.
        let active = state.sort.as_ref().filter(|s| {
            self.columns
                .iter()
                .any(|c| c.is_sortable() && c.name() == s.column)
        });
        let mut heads: Vec<BoxView<'_>> = Vec::with_capacity(self.columns.len());
        for col in &self.columns {
            let label = col.label().to_string();
            let searchable = col.is_searchable();
            let (head_class, aria_sort, header) = if col.is_sortable() {
                let (aria, glyph, next_desc) = match active {
                    Some(s) if s.column == col.name() => (
                        if s.descending {
                            "descending"
                        } else {
                            "ascending"
                        },
                        if s.descending { "\u{2193}" } else { "\u{2191}" },
                        // toggling the active column flips the direction
                        !s.descending,
                    ),
                    _ => ("none", "\u{2195}", false),
                };
                let group_name = self.effective_group_name(state);
                let href = build_url(
                    path,
                    &[
                        ("q", state.search.as_deref()),
                        ("sort", Some(col.name())),
                        ("dir", Some(if next_desc { "desc" } else { "asc" })),
                        ("filters", state.filters_param().as_deref()),
                        ("group_by", group_name.as_deref()),
                    ],
                );
                let aria_label = format!(
                    "Sort by {} {}",
                    label,
                    if next_desc { "descending" } else { "ascending" }
                );
                (
                    "cursor-pointer hover:bg-foreground/5",
                    Some(aria),
                    view! {
                        cx =>
                        <a
                            href=(href)
                            aria-label=(aria_label)
                            class="inline-flex items-center gap-1 hover:text-foreground"
                        >
                            (label.clone())
                            <span
                                role="img"
                                aria-hidden="true"
                                class="inline-flex size-4 items-center justify-center align-middle text-base leading-none text-muted-foreground"
                            >
                                (glyph)
                            </span>
                        </a>
                    }
                    .boxed(),
                )
            } else {
                ("", None, view! { cx => (label.clone()) }.boxed())
            };
            heads.push(view! {
                cx =>
                table_head(
                    attrs: attributes! { class=(head_class) aria-sort=(aria_sort) },
                    (header)
                    if searchable {
                        <span
                            role="img"
                            aria-label="Searchable column"
                            class="ml-2 inline-flex size-4 items-center justify-center align-middle text-base leading-none text-muted-foreground"
                        >
                            "\u{2315}"
                        </span>
                    }
                )
            }
            .boxed());
        }
        if with_delete {
            heads.push(view! { cx => table_head("Actions") }.boxed());
        }
        Ok(view! {
            cx =>
            table_header(
                table_row(
                    if with_bulk {
                        table_head(
                            <input
                                type="checkbox"
                                aria-label="Select all rows"
                                data-bulk-select-all=""
                            >
                        )
                    }
                    for h in heads {
                        (h)
                    }
                )
            )
        }
        .boxed())
    }
}

/// One executed page of rows for [`Table::render`].
///
/// For paginated tables build it from toasty's `Page` via
/// [`Self::from_toasty_page`] (which URL-encodes the engine cursors); for
/// unpaginated tables `Vec<M>` converts directly. An absent cursor simply
/// means no Previous/Next link is rendered — the chrome never invents pages.
#[derive(Debug, Clone)]
pub struct TablePage<M> {
    /// The rows of this page.
    pub rows: Vec<M>,
    /// Encoded cursor for the next page (`?after=`), when one exists.
    pub next_cursor: Option<String>,
    /// Encoded cursor for the previous page (`?before=`), when one exists.
    pub prev_cursor: Option<String>,
}

impl<M> From<Vec<M>> for TablePage<M> {
    fn from(rows: Vec<M>) -> Self {
        Self {
            rows,
            next_cursor: None,
            prev_cursor: None,
        }
    }
}

impl<M: toasty::schema::Model> TablePage<M> {
    /// Wrap a toasty cursor-pagination result, encoding its cursors for URLs.
    ///
    /// # Errors
    ///
    /// Errors when a cursor contains a value the URL codec cannot represent
    /// (see `crate::cursor`).
    pub fn from_toasty_page(page: toasty::stmt::Page<M>) -> Result<Self> {
        Ok(Self {
            rows: page.items,
            next_cursor: page
                .next_cursor
                .as_ref()
                .map(crate::cursor::encode)
                .transpose()?,
            prev_cursor: page
                .prev_cursor
                .as_ref()
                .map(crate::cursor::encode)
                .transpose()?,
        })
    }
}

/// Which column the table is currently sorted by, parsed from
/// `?sort=<column>&dir=asc|desc`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sort {
    /// The app-level field name of the column (matches [`Column::name`]).
    pub column: String,
    /// `true` for `dir=desc`.
    pub descending: bool,
}

/// Request-scoped table state, parsed from the current URL query.
///
/// The single parse point shared by loaders (the search term, ordering via
/// [`Table::order_bys_for_state`]) and render (active sort, toolbar values,
/// pagination links), so the URL is the one truth for list state. The fixed parameter
/// names assume one table per page — per-table prefixes are deferred until a
/// real page needs two tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableState {
    /// `?q=` — trimmed; `None` when absent or blank.
    pub search: Option<String>,
    /// `?sort=` + `?dir=` — `None` when absent or blank.
    pub sort: Option<Sort>,
    /// `?after=` — encoded forward cursor.
    pub after: Option<String>,
    /// `?before=` — encoded backward cursor.
    pub before: Option<String>,
    /// `?filters=` — `key:value,key2:value2` (comma-separated, colon-delimited).
    pub filters: HashMap<String, String>,
    /// `?group_by=` — field name to group by (in-memory, `count` summarizer).
    pub group_by: Option<String>,
}

impl TableState {
    /// Parse the state from the request in `cx`.
    ///
    /// A blank or unknown query parses as neutral state rather than failing
    /// the request. Duplicate keys (`?filters=a&filters=b`) resolve to the
    /// first occurrence: the previous serde decode rejected duplicates, and
    /// swallowing that error as empty state silently dropped every filter —
    /// including export's fail-closed guard (GH #93). Cursor errors still
    /// surface later, at decode time, where they are precise. Renders
    /// without a request context (e.g. unit tests) get neutral state.
    pub fn from_cx(cx: &Cx) -> Self {
        let Some(parts) = topcoat::context::try_request_context::<http::request::Parts>(cx) else {
            return Self::default();
        };
        let params = first_wins_query_params(parts.uri.query().unwrap_or(""));
        let get = |key: &str| params.get(key).map(String::as_str);
        let non_empty = |v: Option<&str>| {
            v.map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        };
        Self {
            search: non_empty(get("q")),
            sort: non_empty(get("sort")).map(|column| Sort {
                column,
                descending: get("dir") == Some("desc"),
            }),
            after: non_empty(get("after")),
            before: non_empty(get("before")),
            filters: get("filters").map(parse_filters_param).unwrap_or_default(),
            group_by: non_empty(get("group_by")),
        }
    }

    /// Serialized `filters` for URL (`key:value,key2:value2`), or `None` when empty.
    ///
    /// Keys/values escape `%`, `:`, `,` (`%25`/`%3A`/`%2C`, GH #93) so a
    /// free-text value like `a,b` round-trips instead of splitting.
    pub fn filters_param(&self) -> Option<String> {
        if self.filters.is_empty() {
            None
        } else {
            let mut pairs: Vec<String> = self
                .filters
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        encode_filter_component(k),
                        encode_filter_component(v)
                    )
                })
                .collect();
            pairs.sort();
            Some(pairs.join(","))
        }
    }

    /// List URL preserving the full table state for the streamed retry link
    /// (GH #98): a filtered/sorted/paginated failure retries the same evidence,
    /// not the bare list.
    pub(crate) fn retry_url(&self, path: &str) -> String {
        let dir = self
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" });
        let filters = self.filters_param();
        build_url(
            path,
            &[
                ("q", self.search.as_deref()),
                ("sort", self.sort.as_ref().map(|s| s.column.as_str())),
                ("dir", dir),
                ("filters", filters.as_deref()),
                ("group_by", self.group_by.as_deref()),
                ("after", self.after.as_deref()),
                ("before", self.before.as_deref()),
            ],
        )
    }

    /// Rebuild list state from live-search shard args (GH #74).
    ///
    /// Shard requests hit `POST /_topcoat/runtime/shards/...`, so
    /// [`Self::from_cx`] would see the endpoint URI — not the list page's
    /// query. The page passes its (static) filter/sort state plus the live
    /// `q` signal explicitly. Live search resets pagination (`after`/`before`
    /// are always `None` — a new search is a new result set, same as the GET
    /// toolbar) and keeps the page's `group_by`.
    pub fn from_live_args(
        q: &str,
        filters_param: &str,
        sort: &str,
        dir: &str,
        group_by: &str,
    ) -> Self {
        let search = {
            let t = q.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        };
        let sort = {
            let c = sort.trim();
            if c.is_empty() {
                None
            } else {
                Some(Sort {
                    column: c.to_string(),
                    descending: dir.trim() == "desc",
                })
            }
        };
        let non_empty = |v: &str| {
            let t = v.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        };
        Self {
            search,
            sort,
            after: None,
            before: None,
            filters: parse_filters_param(filters_param),
            group_by: non_empty(group_by),
        }
    }
}

/// Query-string pairs with the first occurrence winning.
///
/// Deliberately not serde's derived `duplicate_field` behavior: a duplicate
/// `?filters=` used to fail the whole decode, and `from_cx` swallowed that
/// error as empty state — silently dropping filters and export's fail-closed
/// guard (GH #93). Unknown keys are ignored, like the typed decode was.
fn first_wins_query_params(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if !out.contains_key(key.as_ref()) {
            out.insert(key.into_owned(), value.into_owned());
        }
    }
    out
}

/// Parse `filters` query param: `key:value,key2:value2` (trimmed, blank ignored).
///
/// `,`/`:`/`%` inside keys/values are `%`-escaped by [`TableState::filters_param`]
/// (GH #93); decoding restores them. Duplicate keys keep the first occurrence
/// instead of silent last-wins.
fn parse_filters_param(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Split on the first *unescaped* colon: `%3A` stays inside the key/value,
        // so a plain `split_once(':')` is correct on the encoded form.
        if let Some((k_enc, v_enc)) = part.split_once(':') {
            let k = decode_filter_component(k_enc.trim());
            let v = decode_filter_component(v_enc.trim());
            if !k.is_empty() && !v.is_empty() && !map.contains_key(&k) {
                map.insert(k, v);
            }
        }
    }
    map
}

/// Escape `%`, `:`, `,` inside a filter key/value (GH #93).
fn encode_filter_component(s: &str) -> String {
    s.replace('%', "%25")
        .replace(':', "%3A")
        .replace(',', "%2C")
}

/// Decode [`encode_filter_component`] (case-insensitive hex, single pass).
fn decode_filter_component(s: &str) -> String {
    s.replace("%2C", ",")
        .replace("%2c", ",")
        .replace("%3A", ":")
        .replace("%3a", ":")
        .replace("%25", "%")
}

/// Percent-encode a query parameter value (`unreserved` RFC 3986 set passes).
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-encode a single path segment (GH #96).
///
/// Row keys are `String` by contract, so `/`, `?`, `#`, `%`, `+` inside a key
/// must not rewrite the action URL. Topcoat's `path_param_segment` returns
/// the percent-decoded segment, so this round-trips; UUID keys pass through
/// unchanged.
fn encode_path_segment(value: &str) -> String {
    encode_query_value(value)
}

/// FNV-1a (32-bit): stable across runs and Rust versions, no dependency.
/// Used only to disambiguate DOM ids, never for anything security-relevant.
fn fnv1a_32(s: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in s.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Stable DOM id for a table row (GH #104): Topcoat's morph (#392) follows
/// elements by `id` across reruns, so reorderable row content needs one in
/// addition to the keyed-diff `key:`. Derived from the row key (stable for
/// the record, unlike a loop index), sanitized to an HTML-safe token plus a
/// short hash: distinct keys (`Ada Lovelace`, `Ada-Lovelace`) can sanitize to
/// the same token, and duplicate DOM ids would make the morph follow one row.
fn row_dom_id(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 14);
    out.push_str("row-");
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.') {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    out.push_str(&format!("-{:08x}", fnv1a_32(key)));
    out
}

/// Encode live-search snapshot args into one bundle string (GH #104):
/// filters/sort/dir/group_by snapshots travel as a single shard arg so the
/// wire arity stays small. Decoded by [`decode_live_rest`] with the same
/// urlencoding the list toolbar speaks (delimiters round-trip).
pub(crate) fn encode_live_rest(
    q_initial: &str,
    filters: &str,
    sort: &str,
    dir: &str,
    group_by: &str,
) -> String {
    let mut ser = form_urlencoded::Serializer::new(String::new());
    ser.append_pair("q", q_initial);
    ser.append_pair("filters", filters);
    ser.append_pair("sort", sort);
    ser.append_pair("dir", dir);
    ser.append_pair("group_by", group_by);
    ser.finish()
}

/// Decode an [`encode_live_rest`] bundle. Unknown keys are ignored; missing
/// keys decode as empty (matching [`TableState::from_live_args`] blanks).
pub(crate) fn decode_live_rest(rest: &str) -> (String, String, String, String, String) {
    let mut q_initial = String::new();
    let mut filters = String::new();
    let mut sort = String::new();
    let mut dir = String::new();
    let mut group_by = String::new();
    for (k, v) in form_urlencoded::parse(rest.as_bytes()) {
        match &*k {
            "q" => q_initial = v.into_owned(),
            "filters" => filters = v.into_owned(),
            "sort" => sort = v.into_owned(),
            "dir" => dir = v.into_owned(),
            "group_by" => group_by = v.into_owned(),
            _ => {}
        }
    }
    (q_initial, filters, sort, dir, group_by)
}

/// Build `path?k=v&…` from ordered optional parameters, skipping `None`.
pub(crate) fn build_url(path: &str, params: &[(&str, Option<&str>)]) -> String {
    let query = params
        .iter()
        .filter_map(|(k, v)| v.map(|v| format!("{k}={}", encode_query_value(v))))
        .collect::<Vec<_>>()
        .join("&");
    if query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query}")
    }
}

/// Which pages a `Resource` exposes.
#[derive(Debug, Default)]
pub struct Pages<R> {
    _marker: PhantomData<R>,
}

impl<R> Pages<R> {
    /// The conventional CRUD set (list / create / edit / view). Phase 1: stub.
    pub fn crud() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Predicate deciding whether a [`NavigationItem`] matches the request URI —
/// factored out of `NavigationItem` so the field signature stays readable.
pub type HrefCheck = Arc<dyn Fn(&Cx) -> bool + Send + Sync>;

/// Sidebar entry derived from a `Resource` (see `CONTEXT.md`).
#[derive(Default)]
pub struct NavigationItem {
    pub label: String,
    pub url: String,
    pub href_check: Option<HrefCheck>,
    /// Sort key for the sidebar (GH #102): items render in stable `order`
    /// order, so declaration order breaks ties. Resources declare in
    /// `Panel::resource` order (all default `0`); custom items interleave
    /// via [`.sorted()`](Self::sorted) — e.g. `.sorted(-1)` pins above the
    /// resources.
    pub order: i32,
}

impl Clone for NavigationItem {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            url: self.url.clone(),
            href_check: self.href_check.clone(),
            order: self.order,
        }
    }
}

impl std::fmt::Debug for NavigationItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationItem")
            .field("label", &self.label)
            .field("url", &self.url)
            .field("href_check", &self.href_check.is_some())
            .field("order", &self.order)
            .finish()
    }
}

impl PartialEq for NavigationItem {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label && self.url == other.url
    }
}
impl Eq for NavigationItem {}

impl NavigationItem {
    /// Derive a sidebar entry from a `Resource` type, using the given panel
    /// mount prefix.
    ///
    /// Label comes from [`Resource::navigation_label`] (the pluralized model
    /// name), URL from [`Resource::slug`] under the prefix — the same URL
    /// [`crate::panel::Panel::resource`] registers the list page at, so the
    /// sidebar and the router can never disagree.
    pub fn from_resource_with_prefix<R: Resource>(prefix: &str) -> Self {
        let trimmed = prefix.trim_matches('/').trim();
        let base = if trimmed.is_empty() {
            "/admin".to_string()
        } else {
            format!("/{trimmed}")
        };
        Self {
            label: R::navigation_label(),
            url: format!("{base}/{}", R::slug()),
            href_check: None,
            order: 0,
        }
    }

    /// Create a `NavigationItem` from a typed `Href` (e.g. `href!("/admin/showcase")`).
    ///
    /// The label is provided explicitly; `url` is the href's resolved URL
    /// (e.g. `"/admin/showcase"`), and `is_current` delegates to
    /// `Href::is_current` so query/encoding are handled per Topcoat `d273cb15`.
    /// For dynamic `Panel::prefix()` items use `from_resource_with_prefix`.
    pub fn from_href<T, P, Q, F>(
        label: impl Into<String>,
        href: Href<T, P, Q, F>,
        url: impl Into<String>,
    ) -> Self
    where
        T: HrefTarget + Send + Sync + 'static,
        P: HrefParams + Send + Sync + 'static,
        Q: HrefQueries + Send + Sync + 'static,
        F: std::fmt::Display + Send + Sync + 'static,
    {
        // Sidebar sections (e.g. Showcase) should stay active on their
        // sub-pages, while Href::is_current is exact (path + query). Use a
        // slash-boundary prefix check on the href's resolved path so
        // from_href items behave like is_current_path but still benefit from
        // href's encoding-aware path generation.
        let url_string: String = url.into();
        let prefix = url_string.clone();
        let check = Arc::new(move |cx: &Cx| {
            if href.is_current(cx) {
                return true;
            }
            let current = topcoat::router::request::uri(cx).path();
            if current == prefix {
                return true;
            }
            current
                .strip_prefix(prefix.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        }) as Arc<dyn Fn(&Cx) -> bool + Send + Sync>;
        Self {
            label: label.into(),
            url: url_string,
            href_check: Some(check),
            order: 0,
        }
    }

    /// Derive a sidebar entry from a `Resource` type.
    ///
    /// Shorthand for `from_resource_with_prefix::<R>("/admin")` — kept for
    /// single-panel Phase 1 call sites. New code should use
    /// `from_resource_with_prefix` or `Panel::nav_item`.
    pub fn from_resource<R: Resource>() -> Self {
        Self::from_resource_with_prefix::<R>("/admin")
    }

    /// Pin this item's sidebar position (GH #102): lower `order` renders
    /// first, ties keep declaration order.
    pub fn sorted(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Whether this item is current for the request in `cx`.
    ///
    /// If this item was created via `from_href`, delegates to `Href::is_current`
    /// (sorted decoded query + percent-encoding). Otherwise mirrors that
    /// semantics for string URLs: exact path match, or prefix match with slash
    /// boundary for non-root items (so `/admin/showcase` does not false-positive
    /// on `/admin/showcases`), ignoring query. Root `"/admin"` is exact-only
    /// so the Users list is not active on every sub-page (Filament parity).
    pub fn is_current(&self, cx: &Cx) -> bool {
        if let Some(check) = &self.href_check {
            return check(cx);
        }
        let current = topcoat::router::request::uri(cx).path();
        self.is_current_path(current)
    }

    /// Whether this item is current for the given request path (without query):
    /// an exact match, or a prefix match on a slash boundary (so
    /// `/admin/users` is active on `/admin/users/create` but not on
    /// `/admin/userships`). Uniform for every item — since resources mount at
    /// `{prefix}/{slug}` (GH #39), no generated item points at the bare panel
    /// prefix that needed the old root-exact special case.
    ///
    /// Split from `is_current` so `Panel::render_shell` can stay testable
    /// without constructing a full `http::request::Parts` in `Cx`.
    pub fn is_current_path(&self, current_path: &str) -> bool {
        if current_path == self.url {
            return true;
        }
        current_path
            .strip_prefix(&self.url)
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// Maps one Toasty `Model` to its admin UI.
pub trait Resource: Sized + Send + Sync + 'static {
    /// The persisted model this resource administers.
    ///
    /// `Send + Sync` holds for every data-only model struct and is required
    /// for concurrent rendering of the resource's pages.
    type Model: toasty::schema::Model + Send + Sync + 'static;

    /// Whether the current user may view the list page.
    fn can_view_any(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may view the given record.
    ///
    /// Checked on the edit page (GET), the edit POST (which requires both
    /// `can_view` and `can_update`, GH #86), and per row in CSV export. The
    /// list page deliberately checks only `can_view_any` (GH #86): `can_view`
    /// is an in-memory Rust predicate that cannot run in SQL, and filtering
    /// rows after cursor pagination would mislabel pages (holes, wrong
    /// Next/Prev). Row-level visibility that must hold on the list belongs
    /// in [`Self::query`] (the tenancy seam, ADR-0002), which every loader —
    /// list, edit, delete, bulk, export — already funnels through.
    fn can_view(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether the current user may create a new record.
    fn can_create(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may update the given record.
    fn can_update(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether the current user may delete the given record.
    fn can_delete(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether this resource exposes row and bulk delete chrome (GH #96).
    ///
    /// The default renders Delete buttons and the bulk bar; server policy
    /// (`can_delete`) still denies regardless. Read-only resources should
    /// override to `false` so users never reach a 403 after a confirmation
    /// round-trip.
    fn deletable() -> bool {
        true
    }

    /// The URL slug for this resource's pages, e.g. `"users"` mounts the list
    /// at `{panel prefix}/users`.
    ///
    /// Defaults to the Filament convention (`HasRoutes::resolveDefaultSlug`):
    /// take the resource type's name, strip a trailing `Resource`, pluralize
    /// (`UserResource` → `Users`, `CategoryResource` → `Categories`), then
    /// kebab-case (`BlogPostResource` → `blog-posts`). Override for irregular
    /// naming the rules cannot guess (`UsersResource` pluralizes to
    /// `userses` — name resources singular, or override).
    fn slug() -> String {
        let name = type_short_name::<Self>();
        let singular = name.strip_suffix("Resource").unwrap_or(name);
        kebab_case(&pluralize(singular))
    }

    /// The sidebar label, e.g. `"Users"`.
    ///
    /// Defaults to the pluralized `Model` type name (Filament's plural model
    /// label): `User` → `Users`, `Category` → `Categories`, `Person` →
    /// `People`. Override for custom wording.
    fn navigation_label() -> String {
        pluralize(type_short_name::<Self::Model>())
    }

    /// Base query — the **single seam** for tenancy/soft-delete scoping
    /// (ADR-0002). Every loader starts from this query.
    ///
    /// Returns the raw typed statement query (the spec's original signature):
    /// raw queries compose generically — `filter`, `order_by`, and
    /// `Paginate::new` are available on the raw form for any `M: Model` —
    /// which is what lets [`crate::panel::Panel`] drive every resource's list
    /// page through one handler. Scoping it via `Model::filter(..)` in an
    /// override stays as ergonomic as before; the wrapper's extra methods are
    /// only needed by hand-written loaders.
    fn query(_cx: &Cx) -> toasty::stmt::Query<List<Self::Model>> {
        toasty::stmt::Query::<List<Self::Model>>::all()
    }

    /// Whether this resource requires a tenant in every handler (GH #87).
    ///
    /// Opt-in and default-open today: `false` preserves the current behavior
    /// (unscoped `Resource::query` default). Resources with a `tenant_id`
    /// column should override to `true` so a missing tenant fails closed
    /// (403) instead of leaking unscoped rows or minting nil-tenant orphans.
    fn requires_tenant() -> bool {
        false
    }

    /// Description of the list view.
    ///
    /// The default is empty, and an empty table **cannot render**: the
    /// default `Resource` is not listable until it declares columns via
    /// `Table::columns(..)` and a row key via `Table::id(..)` (see
    /// [`Table::render`]).
    fn table(_cx: &Cx) -> Table<Self::Model> {
        Table::new()
    }

    /// Description of the form/infolist. Phase 1: stub.
    fn form(_cx: &Cx) -> Schema {
        Schema::empty()
    }

    /// Which pages the resource exposes. Phase 1: the CRUD stub.
    fn pages() -> Pages<Self> {
        Pages::crud()
    }

    /// Sidebar entry for the resource.
    fn navigation() -> NavigationItem {
        NavigationItem::from_resource::<Self>()
    }

    /// Create a new record from form values.
    ///
    /// The `Panel` create handler validates `required`/`email` inline and checks
    /// `Policy::can_create` before calling this, inside a framework-owned
    /// transaction (GH #84): `ex` is the open tx — run every statement
    /// through it (`exec(&mut *ex)`) and never open a second handle, so the
    /// write commits atomically with the handler's checks. The default
    /// implementation returns an error; resources should override to perform
    /// the actual `toasty::create!` (or `Insert`).
    fn create_record(
        _cx: &Cx,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "create not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Update the already-authorized `record` from form values (GH #86).
    ///
    /// The handler loads `record` through the tenancy-scoped query **inside
    /// the framework transaction** and checks `can_view` + `can_update` on
    /// that snapshot before calling this — use the passed record directly,
    /// never re-query by id (re-loading outside the checked snapshot was the
    /// TOCTOU hole). Run writes through `ex`; commit/rollback is the
    /// handler's job. Residual (documented, not fixed): a concurrent
    /// cross-transaction policy flip landing between this tx's snapshot and
    /// its commit is backend-isolation territory, out of scope here.
    fn update_record(
        _cx: &Cx,
        _record: Self::Model,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "update not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Delete the already-authorized `record` (GH #84, #86): same checked-
    /// snapshot contract as [`Self::update_record`] — no re-query, write
    /// through `ex`.
    fn delete_record(
        _cx: &Cx,
        _record: Self::Model,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "delete not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Bulk-delete the already-authorized `records` (GH #84): the handler
    /// fetches through the tenancy-scoped `IN` query inside the framework
    /// transaction and checks `can_delete` on every row before calling this.
    /// Delete them through `ex` — any error rolls the whole batch back, so
    /// mid-loop failures delete zero rows.
    fn bulk_delete_records(
        _cx: &Cx,
        _records: Vec<Self::Model>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "bulk delete not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Hydrate form values from a record for the Edit page.
    /// Default returns empty; resources should override to return field->value
    /// mappings (e.g. `name -> user.name`).
    fn hydrate_form_values(_record: &Self::Model) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// The last segment of a type's full path, e.g.
/// `argentum_core::resource::tests::UserResource` → `UserResource`.
fn type_short_name<T: ?Sized>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.rsplit("::").next().unwrap_or(name)
}

/// Pluralize a capitalized English word with a compact ruleset (Filament
/// pluralizes via Laravel's `Str::plural`; this is the admin-grade subset):
/// a small irregular table (`person` → `people`, …), consonant-`y` → `ies`
/// (`Category` → `Categories`), sibilant endings → `es` (`Box` → `Boxes`),
/// `f`/`fe` → `ves` (`Knife` → `Knives`) with a few `+s` exceptions, and the
/// default `+s`.
fn pluralize(word: &str) -> String {
    if word.is_empty() {
        return word.to_string();
    }
    let lower = word.to_lowercase();
    const IRREGULAR: &[(&str, &str)] = &[
        ("person", "people"),
        ("man", "men"),
        ("woman", "women"),
        ("child", "children"),
        ("mouse", "mice"),
        ("goose", "geese"),
        ("foot", "feet"),
        ("tooth", "teeth"),
        ("datum", "data"),
        ("criterion", "criteria"),
        ("index", "indices"),
        ("matrix", "matrices"),
        ("vertex", "vertices"),
        ("axis", "axes"),
        ("crisis", "crises"),
        ("analysis", "analyses"),
    ];
    if let Some((_, plural)) = IRREGULAR.iter().find(|(singular, _)| *singular == lower) {
        return match word.chars().next() {
            Some(first) if first.is_uppercase() => capitalize(plural),
            _ => (*plural).to_string(),
        };
    }
    // `f`/`fe` → `ves`, except the words that simply take `s`.
    const F_EXCEPTIONS: &[&str] = &["roof", "chief", "belief", "chef", "cliff", "cuff"];
    const UNCOUNTABLE: &[&str] = &[
        "fish",
        "sheep",
        "deer",
        "moose",
        "series",
        "species",
        "news",
        "equipment",
        "information",
        "rice",
    ];
    if UNCOUNTABLE.contains(&lower.as_str()) {
        return word.to_string();
    }
    if F_EXCEPTIONS.contains(&lower.as_str()) {
        format!("{word}s")
    } else if lower.ends_with('f') {
        format!("{}ves", &word[..word.len() - 1])
    } else if lower.ends_with("fe") {
        format!("{}ves", &word[..word.len() - 2])
    } else if lower.ends_with('y')
        && word.len() > 1
        && !"aeiou".contains(word.chars().nth(word.len() - 2).unwrap_or(' '))
    {
        format!("{}ies", &word[..word.len() - 1])
    } else if ["s", "ss", "sh", "ch", "x", "z"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

/// Convert a CamelCase identifier to kebab-case: `BlogPost` → `blog-post`,
/// `APIKey` → `api-key`.
fn kebab_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 8);
    for (i, &current) in chars.iter().enumerate() {
        if current.is_uppercase() {
            let boundary = i > 0
                && (chars[i - 1].is_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || (chars[i - 1].is_uppercase()
                        && chars.get(i + 1).is_some_and(|next| next.is_lowercase())));
            if boundary {
                out.push('-');
            }
            out.extend(current.to_lowercase());
        } else {
            out.push(current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty::Db;
    use topcoat::context::CxTestBuilder;

    #[derive(Debug, Clone, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct Task {
        #[key]
        #[auto]
        id: uuid::Uuid,
        title: String,
        status: String,
        featured: bool,
        created_at: jiff::Timestamp,
    }

    struct UserResource;

    impl Resource for UserResource {
        type Model = User;

        fn query(_cx: &Cx) -> toasty::stmt::Query<List<User>> {
            // Custom scoping example: only users named Ada
            toasty::stmt::Query::<List<User>>::all().filter(User::fields().name().eq("Ada"))
        }
    }

    struct BareResource;

    impl Resource for BareResource {
        type Model = User;
    }

    #[test]
    fn resource_associated_model_is_accessible() {
        fn assert_resource<R: Resource>() {}
        assert_resource::<UserResource>();
        assert_resource::<BareResource>();
    }

    #[test]
    fn navigation_derives_label_and_url_from_model() {
        let item = NavigationItem::from_resource::<UserResource>();
        // Label: pluralized model name; URL: panel prefix + resource slug
        // (the same URL Panel::resource registers the list page at).
        assert_eq!(item.label, "Users");
        assert_eq!(item.url, "/admin/users");
    }

    #[test]
    fn slugs_follow_the_filament_convention() {
        // UserResource → strip "Resource" → pluralize → kebab-case
        assert_eq!(<UserResource as Resource>::slug(), "users");
        assert_eq!(UserResource::navigation_label(), "Users");
    }

    #[test]
    fn pluralize_and_kebab_follow_english_rules() {
        use super::{kebab_case, pluralize};
        // rules
        assert_eq!(pluralize("User"), "Users");
        assert_eq!(pluralize("Category"), "Categories");
        assert_eq!(pluralize("Dummy"), "Dummies");
        assert_eq!(pluralize("Day"), "Days");
        assert_eq!(pluralize("Box"), "Boxes");
        assert_eq!(pluralize("Bus"), "Buses");
        assert_eq!(pluralize("Church"), "Churches");
        assert_eq!(pluralize("Knife"), "Knives");
        assert_eq!(pluralize("Roof"), "Roofs");
        // irregulars (case preserved)
        assert_eq!(pluralize("Person"), "People");
        assert_eq!(pluralize("Child"), "Children");
        assert_eq!(pluralize("Index"), "Indices");
        // kebab
        assert_eq!(kebab_case("Users"), "users");
        assert_eq!(kebab_case("BlogPost"), "blog-post");
        assert_eq!(kebab_case("APIKey"), "api-key");
    }

    #[test]
    fn navigation_item_is_current_path() {
        let users = NavigationItem {
            label: "Users".to_string(),
            url: "/admin/users".to_string(),
            href_check: None,
            order: 0,
        };
        let showcase = NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
            href_check: None,
            order: 0,
        };
        // exact
        assert!(users.is_current_path("/admin/users"));
        assert!(showcase.is_current_path("/admin/showcase"));
        // slash-boundary — sub-pages active
        assert!(users.is_current_path("/admin/users/create"));
        assert!(showcase.is_current_path("/admin/showcase/table"));
        // slash-boundary — near-misses inactive
        assert!(!users.is_current_path("/admin/userships"));
        assert!(!showcase.is_current_path("/admin/showcases"));
        assert!(!showcase.is_current_path("/admin/showcase-table"));
        // unrelated
        assert!(!users.is_current_path("/other"));
        assert!(!showcase.is_current_path("/admin/users"));
    }

    #[test]
    fn navigation_item_is_current_via_cx() {
        let item = NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
            href_check: None,
            order: 0,
        };
        let (parts, ()) = http::Request::builder()
            .uri("/admin/showcase/table")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert!(item.is_current(&cx));
        let (parts2, ()) = http::Request::builder()
            .uri("/admin/showcases")
            .body(())
            .unwrap()
            .into_parts();
        let cx2 = CxTestBuilder::new().request_context(parts2).build();
        assert!(!item.is_current(&cx2));
    }

    #[test]
    fn default_query_returns_all() {
        let cx = CxTestBuilder::new().build();
        let _q = BareResource::query(&cx);
        // No panic — the default impl returns Model::all()
        let _q2 = UserResource::query(&cx);
    }

    #[test]
    fn table_form_pages_have_defaults() {
        let cx = CxTestBuilder::new().build();
        let _table = UserResource::table(&cx);
        let _form = UserResource::form(&cx);
        let _pages = UserResource::pages();
        let _nav = UserResource::navigation();
    }

    #[tokio::test]
    async fn query_seam_is_cloneable_via_db_helper() {
        // Proves the seam can be combined with the `db(cx)` helper from T2
        // without taking ownership of the query.
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(User { name: "Ada" })
            .exec(&mut db)
            .await
            .unwrap();
        toasty::create!(User { name: "Bob" })
            .exec(&mut db)
            .await
            .unwrap();

        let cx = CxTestBuilder::new().app_context(db).build();
        let mut db = crate::db::db(&cx);
        let rows = UserResource::query(&cx).exec(&mut db).await.unwrap();
        // Custom query filters to Ada only
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");

        let rows_all = BareResource::query(&cx).exec(&mut db).await.unwrap();
        assert_eq!(rows_all.len(), 2);
    }

    #[test]
    fn text_column_searchable_produces_starts_with() {
        let col = TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable();
        assert!(
            col.to_search_expr("Ada").is_some(),
            "searchable should produce expr"
        );
        assert!(
            TextColumn::r#for(User::fields().name(), |u| u.name.clone())
                .to_search_expr("Ada")
                .is_none(),
            "non-searchable should be None"
        );
    }

    #[test]
    fn text_column_sortable_produces_order_by() {
        let col = TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable();
        assert!(
            col.to_order_by(false).is_some(),
            "sortable should produce order_by"
        );
        assert!(
            TextColumn::r#for(User::fields().name(), |u| u.name.clone())
                .to_order_by(false)
                .is_none(),
            "non-sortable should be None"
        );
    }

    #[test]
    fn text_column_renders_cells_via_typed_projection() {
        let plain = TextColumn::r#for(User::fields().name(), |u| u.name.clone());
        let decorated = TextColumn::r#for(User::fields().name(), |u| format!("{}!", u.name));
        let row = User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        };
        assert_eq!(plain.render_cell(&row), "Ada");
        assert_eq!(decorated.render_cell(&row), "Ada!");
        assert_eq!(plain.name(), "name");
        assert_eq!(plain.label(), "Name");
    }

    #[tokio::test]
    async fn table_render_requires_row_key_and_columns() {
        let cx = CxTestBuilder::new().build();
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        // No columns → error
        let no_columns = Table::<User>::r#for(&cx).id(|u| u.id.to_string());
        let page: TablePage<User> = rows.clone().into();
        assert!(
            no_columns.render(&cx, page.clone()).await.is_err(),
            "render without columns must error"
        );
        // Columns but no row key → error (replaces the old panic-on-unknown dispatch)
        let no_key = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(
            no_key.render(&cx, page.clone()).await.is_err(),
            "render without row key must error"
        );
    }

    #[tokio::test]
    async fn paginate_zero_is_a_render_error_not_a_panic() {
        let cx = CxTestBuilder::new().build();
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        // Zero page size is a programmer error (GH #96): a descriptive error
        // the streamed list renders in-region, never a per-request panic.
        let zero = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .paginate(0);
        let page: TablePage<User> = rows.into();
        let err = match zero
            .render_with_state(&cx, page, &TableState::default(), "/admin/users")
            .await
        {
            Ok(_) => panic!("paginate(0) must error"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("per_page > 0"),
            "error must name the contract, got {err}"
        );
    }

    #[tokio::test]
    async fn table_for_columns_renders_with_keyed_rows() {
        let cx = CxTestBuilder::new().build();
        let users_table = Table::<User>::r#for(&cx).id(|u| u.id.to_string()).columns((
            TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable(),
            TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable(),
        ));
        // Use dummy rows for render check (no DB) — keyed by row.id
        let rows = vec![
            User {
                id: uuid::Uuid::new_v4(),
                name: "Ada".to_string(),
            },
            User {
                id: uuid::Uuid::new_v4(),
                name: "Bob".to_string(),
            },
        ];
        let page: TablePage<User> = rows.clone().into();
        let html = users_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // Beautiful chrome: rounded-xl border border-border, table primitives, Token classes
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing table container chrome in {html}"
        );
        assert!(
            html.contains("border-border") && html.contains("text-muted-foreground"),
            "missing Token classes in {html}"
        );
        // searchable indicator ⌕ and sortable indicator ↕ and aria-sort
        assert!(
            html.contains("⌕") || html.contains("search"),
            "missing searchable indicator in {html}"
        );
        assert!(
            html.contains("↕") || html.contains("aria-sort"),
            "missing sortable indicator in {html}"
        );
        assert!(
            html.contains("cursor-pointer"),
            "missing sortable cursor-pointer in {html}"
        );
        assert!(html.contains("Name"), "missing Name header in {html}");
        for row in &rows {
            assert!(
                html.contains(&row.name),
                "missing row name {} in {html}",
                row.name
            );
        }
    }

    #[tokio::test]
    async fn bulk_checkboxes_render_with_keys_and_select_all() {
        let cx = CxTestBuilder::new().build();
        let bulk_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_delete("/admin/users".to_string())
            .with_bulk_delete(true);
        let rows = vec![
            User {
                id: uuid::Uuid::new_v4(),
                name: "Ada".to_string(),
            },
            User {
                id: uuid::Uuid::new_v4(),
                name: "Bob".to_string(),
            },
        ];
        let page: TablePage<User> = rows.clone().into();
        let html = bulk_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // Per-row checkbox carries the row key; header select-all present.
        for row in &rows {
            assert!(
                html.contains(&format!("value=\"{}\"", row.id)),
                "missing checkbox value for {} in {html}",
                row.id
            );
        }
        assert!(
            html.contains("data-row-select"),
            "missing row checkbox marker in {html}"
        );
        assert!(
            html.contains("data-bulk-select-all"),
            "missing select-all in {html}"
        );
        // Bulk form keeps the single `ids` transport + no-JS text fallback.
        assert!(
            html.contains("data-bulk-form"),
            "missing bulk form in {html}"
        );
        assert!(
            html.contains("name=\"ids\"") && html.contains("Bulk Delete"),
            "missing ids fallback in {html}"
        );
        assert!(
            html.contains("data-table-root"),
            "missing table root scope in {html}"
        );

        // Without bulk: no checkboxes, no bulk form.
        let plain = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let page: TablePage<User> = rows.into();
        let html = plain
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("data-row-select") && !html.contains("data-bulk-form"),
            "plain table must not render bulk chrome in {html}"
        );
    }

    #[tokio::test]
    async fn filter_widgets_render_typed_controls() {
        let cx = CxTestBuilder::new().build();
        let table_task1 = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t| {
                t.title.clone()
            }))
            .filters((
                SelectFilter::r#for(
                    Task::fields().status(),
                    vec!["draft".to_string(), "published".to_string()],
                ),
                TernaryFilter::r#for(Task::fields().featured()),
                DateFilter::r#for(Task::fields().created_at()),
            ));
        let page: TablePage<Task> = Vec::new().into();
        // State with an active select value pre-selects it.
        let mut filters = HashMap::new();
        filters.insert("status".to_string(), "published".to_string());
        let state = TableState {
            filters,
            ..TableState::default()
        };
        let html = table_task1
            .render_with_state(&cx, page, &state, "/admin/tasks")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-filters-form"),
            "missing filters form in {html}"
        );
        for name in ["status", "featured", "created_at"] {
            assert!(
                html.contains(&format!("data-filter-name=\"{name}\"")),
                "missing control for {name} in {html}"
            );
        }
        // Select options + current selection.
        assert!(
            html.contains("draft") && html.contains("published"),
            "missing select options in {html}"
        );
        assert!(
            html.contains("value=\"published\" selected")
                || html.contains("value=\"published\" selected=\"\""),
            "published should be selected in {html}"
        );
        // Ternary + date controls.
        assert!(
            html.contains("value=\"true\"") && html.contains("value=\"false\""),
            "missing ternary options in {html}"
        );
        assert!(
            html.contains("type=\"date\""),
            "missing date input in {html}"
        );
        // Free-text fallback keeps the composed value.
        assert!(
            html.contains("name=\"filters\"") && html.contains("status:published"),
            "missing free-text fallback in {html}"
        );
    }

    #[tokio::test]
    async fn variant_filter_renders_select_control() {
        let cx = CxTestBuilder::new().build();
        let table_driver1 = Table::<Driver>::r#for(&cx)
            .id(|d| d.id.to_string())
            .columns(TextColumn::r#for(Driver::fields().name(), |d| {
                d.name.clone()
            }))
            .filters(vehicule_filter());
        let page: TablePage<Driver> = Vec::new().into();
        let html = table_driver1
            .render_with_state(&cx, page, &TableState::default(), "/admin/drivers")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-filter-name=\"vehicule\""),
            "missing variant control in {html}"
        );
        assert!(
            html.contains("Auto") && html.contains("Moto"),
            "missing variant options in {html}"
        );
    }

    #[test]
    fn from_live_args_builds_state() {
        let state = TableState::from_live_args(
            "  Ada ",
            "status:published, featured:true",
            "name",
            "desc",
            "",
        );
        assert_eq!(state.search.as_deref(), Some("Ada"));
        assert_eq!(
            state.filters.get("status").map(String::as_str),
            Some("published")
        );
        assert_eq!(
            state.filters.get("featured").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            state.sort,
            Some(Sort {
                column: "name".to_string(),
                descending: true,
            })
        );
        assert!(state.after.is_none() && state.before.is_none());
        // Blank inputs → neutral state.
        assert_eq!(
            TableState::from_live_args("", "", "", "", ""),
            TableState::default()
        );
    }

    #[tokio::test]
    async fn render_with_state_matches_render() {
        let cx = CxTestBuilder::new().build();
        let table_user1 = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let html_render = table_user1
            .render(&cx, rows.clone().into())
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let html_state = table_user1
            .render_with_state(&cx, rows.into(), &TableState::default(), "")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert_eq!(
            normalize_attrs(&html_render),
            normalize_attrs(&html_state),
            "same table must render the same markup (attribute order excluded, GH #104)"
        );
    }

    /// Sort attributes within each tag for HTML comparison (GH #104):
    /// Topcoat's `Attributes` is a `HashMap`, so spread-merged attributes
    /// (e.g. `<tr>` class + row id) render in nondeterministic order.
    /// Attribute order is semantically irrelevant in HTML.
    fn normalize_attrs(html: &str) -> String {
        fn tag_end(s: &str) -> Option<usize> {
            let mut in_single = false;
            let mut in_double = false;
            for (i, c) in s.char_indices() {
                match c {
                    '\'' if !in_double => in_single = !in_single,
                    '"' if !in_single => in_double = !in_double,
                    '>' if !in_single && !in_double => return Some(i),
                    _ => {}
                }
            }
            None
        }
        let mut out = String::with_capacity(html.len());
        let mut rest = html;
        while let Some(lt) = rest.find('<') {
            out.push_str(&rest[..=lt]);
            rest = &rest[lt + 1..];
            // Pass comments through untouched (their payload is opaque).
            if let Some(comment) = rest.strip_prefix("!--") {
                let end = comment.find("-->").map(|i| i + 3).unwrap_or(comment.len());
                out.push_str(&rest[..3 + end]);
                rest = &rest[3 + end..];
                continue;
            }
            let Some(gt) = tag_end(rest) else {
                out.push_str(rest);
                break;
            };
            let (tag, tail) = rest.split_at(gt);
            out.push_str(&sort_tag_attrs(tag));
            out.push('>');
            rest = &tail[1..];
        }
        out.push_str(rest);
        out
    }

    /// Sort one tag's `name="value"` pairs by name, keeping the tag head.
    fn sort_tag_attrs(tag: &str) -> String {
        let mut parts = Vec::new();
        let rest = tag.trim_start();
        // Tag head (name, `/` for close tags) passes through first.
        let head_len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        let (head, mut tail) = rest.split_at(head_len);
        parts.push(head.to_string());
        tail = tail.trim_start();
        while !tail.is_empty() {
            // Attribute name runs to `=` or whitespace (boolean attr).
            let name_len = tail
                .find(|c: char| c == '=' || c.is_whitespace())
                .unwrap_or(tail.len());
            let (name, after) = tail.split_at(name_len);
            let after = after.trim_start();
            if let Some(value) = after.strip_prefix('=') {
                let value = value.trim_start();
                let (val, len) = if let Some(q) = value.chars().next() {
                    if q == '"' || q == '\'' {
                        let end = value[1..].find(q).map(|i| i + 2).unwrap_or(value.len());
                        (value[..end].to_string(), end)
                    } else {
                        let end = value
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(value.len());
                        (value[..end].to_string(), end)
                    }
                } else {
                    (String::new(), 0)
                };
                parts.push(format!("{name}={val}"));
                tail = value[len..].trim_start();
            } else {
                parts.push(name.to_string());
                tail = after;
            }
        }
        let (head, mut attrs) = (parts.remove(0), parts);
        attrs.sort();
        if attrs.is_empty() {
            head
        } else {
            format!("{head} {}", attrs.join(" "))
        }
    }

    #[test]
    fn normalize_attrs_ignores_attribute_order() {
        // GH #104: Topcoat's HashMap-backed Attributes render spread-merged
        // attributes in nondeterministic order; comparison must not care.
        assert_eq!(
            normalize_attrs(r#"<tr class="a" id="b">x</tr>"#),
            normalize_attrs(r#"<tr id="b" class="a">x</tr>"#)
        );
        assert_eq!(
            normalize_attrs(r#"<input disabled type="x" value="a>b">"#),
            r#"<input disabled type="x" value="a>b">"#.to_string()
        );
        assert!(normalize_attrs("<!--c--><p>plain</p>") == "<!--c--><p>plain</p>");
    }

    #[tokio::test]
    async fn empty_with_filters_shows_filtered_message() {
        let cx = CxTestBuilder::new().build();
        let table_task2 = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t| {
                t.title.clone()
            }))
            .filters(SelectFilter::r#for(
                Task::fields().status(),
                vec!["draft".to_string()],
            ));
        let mut filters = HashMap::new();
        filters.insert("status".to_string(), "draft".to_string());
        let state = TableState {
            filters,
            ..TableState::default()
        };
        let html = table_task2
            .render_with_state(&cx, Vec::new().into(), &state, "/admin/tasks")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("No results for these filters"),
            "filter-only empty must be distinct in {html}"
        );
        assert!(
            html.contains("Clear filters"),
            "filter-only empty needs a clear link in {html}"
        );
    }

    #[tokio::test]
    async fn table_search_filters_via_column() {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(User { name: "Ada" })
            .exec(&mut db)
            .await
            .unwrap();
        toasty::create!(User { name: "Bob" })
            .exec(&mut db)
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let col = TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable();
        let expr = col.to_search_expr("Ada").unwrap();
        let mut db = crate::db::db(&cx);
        let rows = User::filter(expr).exec(&mut db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");
        // Empty term → None
        assert!(col.to_search_expr("").is_none());
        assert!(col.to_search_expr("   ").is_none());
    }

    #[test]
    fn table_search_expr_ors_across_searchable_columns() {
        let cx = CxTestBuilder::new().build();
        let users_table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable(),
            TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable(),
        ));
        assert!(users_table.search_expr("Ada").is_some());
        assert!(users_table.search_expr("").is_none());
        assert!(users_table.search_expr("   ").is_none());
        let table_none = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(table_none.search_expr("Ada").is_none());
    }

    #[test]
    fn table_order_by_returns_first_sortable() {
        let cx = CxTestBuilder::new().build();
        let users_table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable(),
            TextColumn::r#for(User::fields().name(), |u| u.name.clone()),
        ));
        assert!(users_table.order_by(false).is_some());
        let table_none = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(table_none.order_by(false).is_none());
    }

    #[test]
    fn table_order_bys_single_sort_column() {
        let cx = CxTestBuilder::new().build();
        let users_table = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());
        let orders = users_table.order_bys();
        // Single sortable column, no app-level PK suffix — toasty's engine
        // appends the physical PK columns to ambiguous cursor orderings
        // internally (GH #76).
        assert_eq!(orders.len(), 1, "sortable column only, got {orders:?}");
        // No sortable → empty
        let table_none = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(
            table_none.order_bys().is_empty(),
            "non-sortable should have no order_bys"
        );
    }

    fn cx_with_query(query: &str) -> Cx {
        let uri = if query.is_empty() {
            "/admin".to_string()
        } else {
            format!("/admin?{query}")
        };
        let (parts, ()) = http::Request::builder()
            .uri(uri)
            .body(())
            .unwrap()
            .into_parts();
        CxTestBuilder::new().request_context(parts).build()
    }

    #[test]
    fn table_state_parses_query_params() {
        let cx = cx_with_query("q=Ada+Lovelace&sort=name&dir=desc&after=abc123");
        let state = TableState::from_cx(&cx);
        assert_eq!(
            state.search.as_deref(),
            Some("Ada Lovelace"),
            "plus must decode to space"
        );
        assert_eq!(
            state.sort,
            Some(Sort {
                column: "name".to_string(),
                descending: true,
            })
        );
        assert_eq!(state.after.as_deref(), Some("abc123"));
        assert!(state.before.is_none());

        // Absent / blank / malformed → neutral state
        let cx = cx_with_query("");
        let state = TableState::from_cx(&cx);
        assert_eq!(state, TableState::default());
        let cx = cx_with_query("q=&sort=&dir=weird");
        let state = TableState::from_cx(&cx);
        assert_eq!(state, TableState::default());
    }

    #[test]
    fn table_state_duplicate_params_keep_first_and_never_fail_open() {
        // GH #93: a duplicate param used to fail the serde decode, and
        // `from_cx` swallowed that as empty state — dropping every filter
        // (and export's fail-closed guard along with it).
        let cx = cx_with_query("filters=status:published&filters=status:draft&q=Ada&q=Grace");
        let state = TableState::from_cx(&cx);
        assert_eq!(
            state.filters.get("status").map(String::as_str),
            Some("published"),
            "first occurrence must win, not vanish"
        );
        assert_eq!(state.search.as_deref(), Some("Ada"));
    }

    #[test]
    fn order_bys_for_state_resolves_sort_param_with_fallbacks() {
        let cx = CxTestBuilder::new().build();
        let sorted = Table::<User>::r#for(&cx)
            .paginate(25)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());

        // ?sort=name&dir=desc → name desc (toasty appends PK internally)
        let state = TableState {
            sort: Some(Sort {
                column: "name".to_string(),
                descending: true,
            }),
            ..TableState::default()
        };
        let orders = sorted.order_bys_for_state(&state);
        assert_eq!(orders.len(), 1, "sort column only, got {orders:?}");

        // Unknown sort column → declared default (name asc)
        let state = TableState {
            sort: Some(Sort {
                column: "nope".to_string(),
                descending: false,
            }),
            ..TableState::default()
        };
        assert_eq!(sorted.order_bys_for_state(&state).len(), 1);

        // No sort at all → declared default
        assert_eq!(sorted.order_bys_for_state(&TableState::default()).len(), 1);

        // Paginated table with no sortable column → PK-only deterministic order
        let unsorted = Table::<User>::r#for(&cx)
            .paginate(25)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let orders = unsorted.order_bys_for_state(&TableState::default());
        assert_eq!(
            orders.len(),
            1,
            "PK-only for paginated unsorted, got {orders:?}"
        );

        // Unpaginated and unsorted → empty (query stays unordered)
        let plain = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(plain.order_bys_for_state(&TableState::default()).is_empty());
    }

    #[tokio::test]
    async fn table_page_round_trips_real_cursors() {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["Ada", "Bob", "Cara"] {
            toasty::create!(User { name }).exec(&mut db).await.unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let users_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());
        let mut db = crate::db::db(&cx);

        // Page 1 of 1-per-page: full page → real next cursor.
        let page1 = users_table
            .order_bys()
            .iter()
            .fold(User::all(), |q, ord| q.order_by(ord.clone()))
            .paginate(1)
            .exec(&mut db)
            .await
            .unwrap();
        let tp1 = TablePage::from_toasty_page(page1).unwrap();
        assert_eq!(tp1.rows.len(), 1);
        assert_eq!(tp1.rows[0].name, "Ada");
        let cursor = tp1.next_cursor.expect("full page has a next cursor");

        // The encoded cursor resumes the walk without skipping tied rows.
        let tp1_decoded = crate::cursor::decode(&cursor).unwrap();
        let page2 = User::all()
            .order_by(User::fields().name().asc())
            .paginate(1)
            .after(tp1_decoded)
            .exec(&mut db)
            .await
            .unwrap();
        let tp2 = TablePage::from_toasty_page(page2).unwrap();
        assert_eq!(tp2.rows[0].name, "Bob", "cursor must resume after Ada");
    }

    #[derive(Debug, Clone, PartialEq, toasty::Embed)]
    enum Vehicule {
        Auto {
            #[shared(puissance)]
            puissance: String,
            seats: String,
        },
        Moto {
            #[shared(puissance)]
            puissance: String,
            cc: String,
        },
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct Driver {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        vehicule: Vehicule,
    }

    fn vehicule_filter() -> VariantFilter<Driver> {
        VariantFilter::new(
            "vehicule",
            "Véhicule",
            vec![
                ("Auto".to_string(), Driver::fields().vehicule().is_auto()),
                ("Moto".to_string(), Driver::fields().vehicule().is_moto()),
            ],
        )
    }

    #[tokio::test]
    async fn date_filter_date_only_matches_whole_day() {
        let mut db = Db::builder()
            .models(toasty::models!(Task))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for (title, ts) in [
            ("Morning", "2024-01-15T09:30:00Z"),
            ("Night", "2024-01-15T23:59:59Z"),
            ("Next", "2024-01-16T00:00:01Z"),
        ] {
            toasty::create!(Task {
                title: title.to_string(),
                status: "draft".to_string(),
                featured: false,
                created_at: ts.parse::<jiff::Timestamp>().unwrap(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let f = DateFilter::r#for(Task::fields().created_at());
        let expr = f.to_expr("2024-01-15").expect("date-only must build");
        let mut db2 = db.clone();
        let mut rows = Task::filter(expr).exec(&mut db2).await.unwrap();
        rows.sort_by(|a, b| a.title.cmp(&b.title));
        assert_eq!(
            rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            vec!["Morning", "Night"],
            "date-only must match the whole UTC day (GH #93)"
        );
        // Exact RFC3339 instants still match exactly.
        let expr = f
            .to_expr("2024-01-15T09:30:00Z")
            .expect("rfc3339 must build");
        let rows = Task::filter(expr).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(f.to_expr("not-a-date").is_none());
    }

    #[test]
    fn date_filter_recovers_plus_offsets_mangled_by_query_decode() {
        let f = DateFilter::r#for(Task::fields().created_at());
        // `+02:00` arrives as ` 02:00` after `+`-as-space decoding (GH #93).
        assert!(f.to_expr("2024-01-15T09:30:00 02:00").is_some());
        assert!(f.to_expr("2024-01-15T09:30:00+02:00").is_some());
        assert!(f.to_expr("not-a-date").is_none());
        assert!(f.to_expr("").is_none());
    }

    #[test]
    fn variant_filter_to_expr_contract() {
        let f = vehicule_filter();
        assert_eq!(f.name(), "vehicule");
        assert_eq!(f.label_str(), "Véhicule");
        assert!(f.to_expr("").is_none(), "empty yields no filter");
        assert!(f.to_expr("   ").is_none(), "blank yields no filter");
        assert!(
            f.to_expr("Avion").is_none(),
            "unknown yields no filter, got {:?}",
            f.to_expr("Avion").is_some()
        );
        assert!(f.to_expr("Auto").is_some(), "known variant must match");
        assert!(f.to_expr("Moto").is_some(), "known variant must match");
        // Whitespace trims like SelectFilter.
        assert!(f.to_expr("  Moto  ").is_some());
        // Via the Filter enum + IntoFilters seam.
        let via_enum: Filter<Driver> = f.clone().into();
        assert_eq!(via_enum.name(), "vehicule");
        assert!(via_enum.to_expr("Moto").is_some());
        assert!(via_enum.to_expr("nope").is_none());
        let vec = f.into_filters();
        assert_eq!(vec.len(), 1);
    }

    #[tokio::test]
    async fn variant_filter_hits_only_the_variant() {
        let mut db = Db::builder()
            .models(toasty::models!(Driver))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        // Same shared `puissance` value in both variants — the variant gate
        // must exclude the other variant (GH #77 acceptance).
        toasty::create!(Driver {
            name: "Alice",
            vehicule: Vehicule::Auto {
                puissance: "80".to_string(),
                seats: "4".to_string(),
            },
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Driver {
            name: "Bob",
            vehicule: Vehicule::Moto {
                puissance: "80".to_string(),
                cc: "600".to_string(),
            },
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Driver {
            name: "Cara",
            vehicule: Vehicule::Auto {
                puissance: "120".to_string(),
                seats: "2".to_string(),
            },
        })
        .exec(&mut db)
        .await
        .unwrap();

        let f = vehicule_filter();
        let mut db2 = db.clone();
        let motos = Driver::filter(f.to_expr("Moto").unwrap())
            .exec(&mut db2)
            .await
            .unwrap();
        assert_eq!(
            motos.len(),
            1,
            "Moto filter must hit one row, got {motos:?}"
        );
        assert_eq!(motos[0].name, "Bob");

        let autos = Driver::filter(f.to_expr("Auto").unwrap())
            .exec(&mut db2)
            .await
            .unwrap();
        assert_eq!(
            autos.len(),
            2,
            "Auto filter must hit two rows, got {autos:?}"
        );

        // Composes with search via AND (the loader's contract).
        let search = Driver::fields().name().starts_with("B".to_string());
        let both = search.and(f.to_expr("Moto").unwrap());
        let rows = Driver::filter(both).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Bob");

        // Same shared value, other variant excluded.
        let both = Driver::fields()
            .name()
            .starts_with("A".to_string())
            .and(f.to_expr("Moto").unwrap());
        let rows = Driver::filter(both).exec(&mut db2).await.unwrap();
        assert!(
            rows.is_empty(),
            "Alice shares puissance 80 but is Auto, must not match Moto: {rows:?}"
        );
    }

    #[test]
    fn to_csv_defuses_formula_cells_per_owasp() {
        let cx = CxTestBuilder::new().build();
        let csv_table = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        for payload in ["=1+1", "+1+1", "-1+1", "@SUM(1+1)", "|id", "%x", "  =cmd"] {
            let rows = vec![User {
                id: uuid::Uuid::nil(),
                name: payload.to_string(),
            }];
            let page: TablePage<User> = rows.into();
            let csv = csv_table.to_csv(&page);
            let body = csv.lines().nth(1).unwrap_or("");
            assert!(
                body.starts_with('\''),
                "formula payload {payload:?} must be defused with leading `'`, got {body:?}"
            );
        }
        // Plain values stay untouched; RFC4180 quoting still applies.
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada, \"the\" first".to_string(),
        }];
        let csv = csv_table.to_csv(&rows.into());
        assert!(
            csv.contains("\"Ada, \"\"the\"\" first\""),
            "quoting broke: {csv:?}"
        );
    }

    #[test]
    #[should_panic(expected = "searchable() on computed column")]
    fn computed_searchable_panics_loudly() {
        let _ = TextColumn::computed("Status", |u: &User| u.name.clone()).searchable();
    }

    #[test]
    #[should_panic(expected = "sortable() on computed column")]
    fn computed_sortable_panics_loudly() {
        let _ = TextColumn::computed("Status", |u: &User| u.name.clone()).sortable();
    }

    #[test]
    fn computed_columns_declare_no_predicate_chrome_agreement() {
        let col = TextColumn::computed("Status", |u: &User| u.name.clone());
        assert!(!col.is_searchable() && !col.is_sortable());
        assert!(col.to_search_expr("x").is_none());
        assert!(col.to_order_by(false).is_none());
    }

    #[test]
    fn path_segment_encoding_keeps_uuids_and_escapes_reserved() {
        let uuid = uuid::Uuid::nil().to_string();
        assert_eq!(encode_path_segment(&uuid), uuid);
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        assert_eq!(encode_path_segment("a+b@c.com"), "a%2Bb%40c.com");
        assert_eq!(encode_path_segment("100%"), "100%25");
        assert_eq!(encode_path_segment("a?b#c"), "a%3Fb%23c");
    }

    #[test]
    fn row_dom_ids_are_stable_and_html_safe() {
        // GH #104: morph follows `id`s across reruns — derived from the row
        // key (record-stable), never a loop index, sanitized to tokens.
        assert_eq!(
            row_dom_id("550e8400-e29b-41d4-a716-446655440000"),
            row_dom_id("550e8400-e29b-41d4-a716-446655440000"),
            "ids must be stable per key"
        );
        let uuid_id = row_dom_id("550e8400-e29b-41d4-a716-446655440000");
        assert!(uuid_id.starts_with("row-550e8400-e29b-41d4-a716-446655440000-"));
        assert!(
            uuid_id.is_ascii(),
            "id stays an ASCII token, got {uuid_id:?}"
        );
        // Keys that sanitize to the same token must not collide.
        assert_ne!(row_dom_id("Ada Lovelace"), row_dom_id("Ada-Lovelace"));
        assert_ne!(row_dom_id("a/b?c"), row_dom_id("a-b-c"));
    }

    #[tokio::test]
    async fn rendered_rows_carry_stable_dom_ids() {
        // GH #104: every rendered row exposes its morph id; re-rendering the
        // same page yields the same ids.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let rows = vec![
            User {
                id: uuid::Uuid::nil(),
                name: "Ada".to_string(),
            },
            User {
                id: uuid::Uuid::max(),
                name: "Alan".to_string(),
            },
        ];
        let render = async |rows: Vec<User>| {
            tbl.render_with_state(&cx, rows.into(), &TableState::default(), "/admin/users")
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(&cx)
        };
        let first = render(rows.clone()).await;
        assert!(
            first.contains("id=\"row-00000000-0000-0000-0000-000000000000-"),
            "missing morph id for first row, got {first}"
        );
        assert!(
            first.contains("id=\"row-ffffffff-ffff-ffff-ffff-ffffffffffff-"),
            "missing morph id for second row, got {first}"
        );
        let second = render(rows).await;
        assert_eq!(
            first.matches("id=\"row-").count(),
            second.matches("id=\"row-").count(),
            "reruns must keep stable row ids"
        );
    }

    #[test]
    fn filters_param_round_trips_reserved_chars() {
        let mut filters = HashMap::new();
        filters.insert("q".to_string(), "a,b".to_string());
        filters.insert("tag".to_string(), "x:y%z".to_string());
        let state = TableState {
            filters,
            ..TableState::default()
        };
        let param = state.filters_param().expect("must serialize");
        assert!(param.contains("%2C") && param.contains("%3A") && param.contains("%25"));
        let back = parse_filters_param(&param);
        assert_eq!(back.get("q").map(String::as_str), Some("a,b"));
        assert_eq!(back.get("tag").map(String::as_str), Some("x:y%z"));
        // Duplicate keys keep the first, never silent last-wins.
        let dup = parse_filters_param("k:a,k:b");
        assert_eq!(dup.get("k").map(String::as_str), Some("a"));
        // Legacy plain values still parse.
        let legacy = parse_filters_param("status:published, featured:true");
        assert_eq!(legacy.get("status").map(String::as_str), Some("published"));
    }

    #[test]
    fn live_rest_bundle_round_trips_delimiters() {
        // GH #104: snapshot args (incl. `% : ,` filter delimiters) survive
        // the shard wire format.
        let rest = encode_live_rest("Ada", "status:a,b", "name", "desc", "status");
        let (q, filters, sort, dir, group_by) = decode_live_rest(&rest);
        assert_eq!(q, "Ada");
        assert_eq!(filters, "status:a,b");
        assert_eq!(sort, "name");
        assert_eq!(dir, "desc");
        assert_eq!(group_by, "status");
        let empty = decode_live_rest("");
        assert_eq!(
            empty,
            (
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string()
            )
        );
    }

    fn status_table(cx: &Cx) -> Table<Task> {
        Table::<Task>::r#for(cx)
            .id(|t| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t| {
                t.title.clone()
            }))
            .filters(SelectFilter::r#for(
                Task::fields().status(),
                vec!["published".to_string(), "draft".to_string()],
            ))
    }

    fn filters_state(pairs: &[(&str, &str)]) -> TableState {
        TableState {
            filters: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..TableState::default()
        }
    }

    #[test]
    fn unapplied_filters_flags_unknown_keys_and_rejected_values() {
        // GH #93: typo'd keys and allowlist-missed values must be visible,
        // never silently unfiltered.
        let cx = CxTestBuilder::new().build();
        let tbl = status_table(&cx);
        assert!(tbl.unapplied_filters(&filters_state(&[])).is_empty());
        assert!(
            tbl.unapplied_filters(&filters_state(&[("status", "published")]))
                .is_empty(),
            "valid filter must apply"
        );
        assert_eq!(
            tbl.unapplied_filters(&filters_state(&[("stauts", "published")])),
            vec![("stauts:published".to_string(), "unknown filter".to_string())]
        );
        assert_eq!(
            tbl.unapplied_filters(&filters_state(&[("status", "Published")])),
            vec![("status:Published".to_string(), "invalid value".to_string())]
        );
    }

    #[tokio::test]
    async fn unknown_filter_renders_alert_banner_and_keeps_200() {
        // GH #93: the list keeps a 200 but warns instead of lying about
        // "these filters".
        let cx = CxTestBuilder::new().build();
        let tbl = status_table(&cx);
        let rows = vec![Task {
            id: uuid::Uuid::nil(),
            title: "Hello".to_string(),
            status: "published".to_string(),
            featured: false,
            created_at: jiff::Timestamp::now(),
        }];
        let html = tbl
            .render_with_state(
                &cx,
                rows.into(),
                &filters_state(&[("stauts", "published")]),
                "/admin/tasks",
            )
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("role=\"alert\"") && html.contains("stauts:published"),
            "typo filter must warn, got {html}"
        );

        let rows = vec![Task {
            id: uuid::Uuid::nil(),
            title: "Hello".to_string(),
            status: "published".to_string(),
            featured: false,
            created_at: jiff::Timestamp::now(),
        }];
        let html = tbl
            .render_with_state(
                &cx,
                rows.into(),
                &filters_state(&[("status", "published")]),
                "/admin/tasks",
            )
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("role=\"alert\""),
            "valid filter must not warn, got {html}"
        );
    }

    #[tokio::test]
    async fn group_by_survives_pager_and_labels_page_local_counts() {
        let cx = CxTestBuilder::new().build();
        let grouped = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable())
            .group_by("status", |u| u.name.clone())
            .paginate(1);
        let state = TableState {
            group_by: Some("status".to_string()),
            sort: Some(Sort {
                column: "name".to_string(),
                descending: false,
            }),
            ..TableState::default()
        };
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let page = TablePage {
            rows,
            next_cursor: Some("abc".to_string()),
            prev_cursor: None,
        };
        let html = grouped
            .render_with_state(&cx, page, &state, "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("on this page"),
            "group header must be page-local, got {html}"
        );
        assert!(
            html.contains("group_by") && html.contains("after=abc"),
            "pager must preserve group_by, got {html}"
        );
    }

    #[tokio::test]
    async fn group_by_unknown_value_renders_no_headers_and_drops_param() {
        // GH #92: `?group_by=` must name the declared group — any other
        // value renders no headers and vanishes from pager links instead of
        // silently grouping by the single declared key.
        let cx = CxTestBuilder::new().build();
        let grouped = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable())
            .group_by("status", |u| u.name.clone())
            .paginate(1);
        let state = TableState {
            group_by: Some("email".to_string()),
            sort: Some(Sort {
                column: "name".to_string(),
                descending: false,
            }),
            ..TableState::default()
        };
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let page = TablePage {
            rows,
            next_cursor: Some("abc".to_string()),
            prev_cursor: None,
        };
        let html = grouped
            .render_with_state(&cx, page, &state, "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("on this page"),
            "unknown group_by must render no headers, got {html}"
        );
        assert!(
            !html.contains("group_by"),
            "unknown group_by must drop from links, got {html}"
        );
    }

    #[tokio::test]
    async fn void_window_links_back_to_first_page() {
        // GH #98: a cursor past the last row (rows deleted under pagination)
        // must offer navigation, never a pager-less dead end.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable())
            .paginate(1);
        let void_page = TablePage {
            rows: Vec::new(),
            next_cursor: None,
            prev_cursor: None,
        };
        let state = TableState {
            after: Some("abc".to_string()),
            sort: Some(Sort {
                column: "name".to_string(),
                descending: false,
            }),
            ..TableState::default()
        };
        let html = tbl
            .render_with_state(&cx, void_page, &state, "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("Back to first page"),
            "void window must link home, got {html}"
        );

        // A genuinely empty first page stays pager-less (its empty-state
        // already offers Clear links).
        let empty_first = TablePage {
            rows: Vec::new(),
            next_cursor: None,
            prev_cursor: None,
        };
        let html = tbl
            .render_with_state(&cx, empty_first, &TableState::default(), "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("Back to first page"),
            "empty first page must stay pager-less, got {html}"
        );
    }

    #[tokio::test]
    async fn skeleton_shares_table_root_and_defer_clears_for_swap() {
        let cx = CxTestBuilder::new().build();
        let deferred = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .defer(true);
        assert!(deferred.is_defer());
        let html = deferred
            .render_skeleton(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-table-root"),
            "skeleton must share table root, got {html}"
        );
        assert!(
            html.contains("aria-busy"),
            "skeleton must announce loading, got {html}"
        );
        assert!(
            html.contains("aria-hidden"),
            "skeleton must hold chrome placeholders, got {html}"
        );
        // The streamed swap renders through a copy with the flag cleared.
        let swapped = deferred.without_skeleton();
        assert!(!swapped.is_defer());
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let html = swapped
            .render_with_state(&cx, rows.into(), &TableState::default(), "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("Ada"),
            "swap payload must be rows, got {html}"
        );
    }

    #[test]
    fn retry_url_preserves_full_table_state() {
        let mut filters = HashMap::new();
        filters.insert("status".to_string(), "published".to_string());
        let state = TableState {
            search: Some("Ada".to_string()),
            sort: Some(Sort {
                column: "name".to_string(),
                descending: true,
            }),
            after: Some("cur".to_string()),
            filters,
            group_by: Some("status".to_string()),
            ..TableState::default()
        };
        let url = state.retry_url("/admin/users");
        for part in [
            "q=Ada",
            "sort=name",
            "dir=desc",
            "filters=",
            "group_by=status",
            "after=cur",
        ] {
            assert!(url.contains(part), "retry must preserve {part}, got {url}");
        }
    }
}
