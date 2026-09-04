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
use serde::Deserialize;
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

/// Date filter — exact match on a `Timestamp` field (e.g. `created_at = "2024-01-15"`).
/// For now exact `Timestamp` equality; range support is future.
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

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        // Accept RFC3339 or YYYY-MM-DD (midnight UTC)
        if let Ok(ts) = v.parse::<jiff::Timestamp>() {
            return Some(self.lens.clone().eq(ts));
        }
        if let Ok(date) = v.parse::<jiff::civil::Date>() {
            let ts = date.to_string() + "T00:00:00Z";
            if let Ok(ts) = ts.parse::<jiff::Timestamp>() {
                return Some(self.lens.clone().eq(ts));
            }
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

/// Filter enum — the `Table::filters` seam.
#[derive(Debug, Clone)]
pub enum Filter<M> {
    Select(SelectFilter<M>),
    Ternary(TernaryFilter<M>),
    Date(DateFilter<M>),
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

impl<M> Filter<M>
where
    M: toasty::schema::Model,
{
    pub fn name(&self) -> &str {
        match self {
            Filter::Select(f) => f.name(),
            Filter::Ternary(f) => f.name(),
            Filter::Date(f) => f.name(),
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Filter::Select(f) => f.label_str(),
            Filter::Ternary(f) => f.label_str(),
            Filter::Date(f) => f.label_str(),
        }
    }
    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        match self {
            Filter::Select(f) => f.to_expr(value),
            Filter::Ternary(f) => f.to_expr(value),
            Filter::Date(f) => f.to_expr(value),
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
/// only way to read a field generically (see `EXTERNAL_GAPS.md` "instance →
/// field-value extraction"). A typo in the closure body fails at compile
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
    /// timestamps, joined values.
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
        self.searchable = true;
        self
    }

    pub fn sortable(mut self) -> Self {
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
/// (see `EXTERNAL_GAPS.md`).
pub type RowKey<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

/// Table description of a `Resource`'s list view. Declares columns and how they map to queries.
///
/// Row identity is mandatory and typed: [`Table::id`] declares the row-key
/// projection and [`Table::render`] errors without it — the old stringly-typed
/// `HasId`/`GetField` dispatch (GH #10) is gone, cells render via
/// [`TextColumn`]'s lens-bound closure where typos fail at compile time
/// instead of panicking at render.
pub type GroupKey<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

pub struct Table<M> {
    columns: Vec<Column<M>>,
    filters: Vec<Filter<M>>,
    group_by: Option<GroupKey<M>>,
    row_key: Option<RowKey<M>>,
    page_size: Option<usize>,
    search_ui: Option<bool>,
    show_skeleton: bool,
    is_boundary: bool,
    defer_initial: bool,
    delete_prefix: Option<String>,
    bulk_delete: bool,
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
    /// falling back to loop indices.
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

    /// Filters as slice (for testing).
    pub fn filter_list(&self) -> &[Filter<M>] {
        &self.filters
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

    /// Group rows in-memory by a key (count summarizer). No GROUP BY SQL.
    pub fn group_by(mut self, key: impl Fn(&M) -> String + Send + Sync + 'static) -> Self {
        self.group_by = Some(Arc::new(key));
        self
    }

    /// Enable real cursor pagination with the given page size.
    ///
    /// Loaders pair this with toasty's `.paginate(per_page)` (via
    /// [`TablePage::from_toasty_page`]); the render then shows Previous/Next
    /// links built from the executed page's cursors — never fake page
    /// numbers. Also implies a deterministic PK ordering when the table
    /// declares no sortable column (see [`Self::order_bys_for_state`]).
    pub fn paginate(mut self, per_page: usize) -> Self {
        assert!(per_page > 0, "pagination page size must be > 0");
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

    /// Enable skeleton placeholder rows (reserved for future `Boundary` `defer`).
    pub fn skeleton(mut self, enabled: bool) -> Self {
        self.show_skeleton = enabled;
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
    /// data loads are `#[memoize]`d so streaming can fill them.
    pub fn defer(mut self, enabled: bool) -> Self {
        self.defer_initial = enabled;
        self.show_skeleton = enabled;
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
    /// Panics if `M` is not a root model: without a primary key there is no
    /// deterministic order, and silent omission would surface as a toasty
    /// "requires an ORDER BY" error under cursor pagination.
    fn pk_order_bys() -> Vec<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        let app_model = M::schema();
        let root = app_model.as_root().unwrap_or_else(|| {
            panic!(
                "pk_order_bys: {} is not a root model; deterministic pagination needs its primary key",
                std::any::type_name::<M>()
            )
        });
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
        let state = TableState::from_cx(cx);
        let path = topcoat::context::try_request_context::<http::request::Parts>(cx)
            .map(|parts| parts.uri.path().to_string())
            .unwrap_or_default();
        let delete_prefix = self.delete_prefix.clone();
        let head = self
            .render_thead(cx, &state, &path, delete_prefix.is_some())
            .await?;
        let show_search = self.search_enabled();
        let search_bar = if show_search {
            Some(self.render_search_bar(cx, &state, &path).await?)
        } else {
            None
        };
        let show_filters = !self.filters.is_empty();
        let filter_bar = if show_filters {
            Some(self.render_filter_bar(cx, &state, &path).await?)
        } else {
            None
        };
        let bulk_bar_view: BoxView<'_> = if self.bulk_delete && self.delete_prefix.is_some() {
            let bulk_action = format!("{}/bulk-delete", self.delete_prefix.clone().unwrap());
            view! {
                cx =>
                <form
                    method="post"
                    action=(bulk_action)
                    class="flex gap-2 p-3 border-b border-border"
                >
                    <input
                        name="ids"
                        placeholder="ids comma-separated"
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
        let pager = self.render_pager(cx, &state, &path, &page).await?;
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

        if page.rows.is_empty() {
            let empty_cell = self
                .render_empty_cell(cx, &state, &path, delete_prefix.is_some())
                .await?;
            let inner = view! {
                cx =>
                <div class="rounded-xl border border-border overflow-hidden">
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

        // Grouping (in-memory, count summarizer) — when `?group_by=` is present and table has a group key.
        // Rendered after skeleton/empty so defer shows skeleton and empty shows
        // the honest empty state even when `?group_by=` is set (GH #75).
        if let (Some(group_fn), Some(_)) = (&self.group_by, &state.group_by) {
            use std::collections::BTreeMap;
            let mut groups: BTreeMap<String, usize> = BTreeMap::new();
            for row in &page.rows {
                *groups.entry(group_fn(row)).or_insert(0) += 1;
            }
            let mut group_views: Vec<BoxView<'_>> = Vec::new();
            for (key, count) in groups {
                let text = format!("{} ({})", key, count);
                group_views.push(
                    view! {
                        cx =>
                        <div class="px-4 py-2 bg-muted text-sm font-medium">(text)</div>
                    }
                    .boxed(),
                );
            }
            let inner = view! {
                cx =>
                <div class="rounded-xl border border-border overflow-hidden">
                    if show_search {
                        (search_bar.expect("search bar built when enabled"))
                    }
                    if show_filters {
                        (filter_bar.expect("filter bar built when enabled"))
                    }
                    (bulk_bar_view)
                    for gv in group_views {
                        (gv)
                    }
                    table(
                        (head)
                        table_body(
                            for (key, cells) in &row_data {
                                let key_for_row = key.clone();
                                let key_for_action = key.clone();
                                table_row(
                                    key: key_for_row,
                                    for cell in cells {
                                        table_cell((cell.clone()))
                                    }
                                    if let Some(prefix) = &delete_prefix {
                                        table_cell(
                                            <form
                                                method="post"
                                                action=(format!("{}/{}/delete", prefix, key_for_action))
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
            if is_boundary {
                return Ok(view! { cx => <div data-boundary="table">(inner)</div> }.boxed());
            } else {
                return Ok(inner.boxed());
            }
        }

        let inner = view! {
            cx =>
            <div class="rounded-xl border border-border overflow-hidden">
                if show_search {
                    (search_bar.expect("search bar built when enabled"))
                }
                if show_filters {
                    (filter_bar.expect("filter bar built when enabled"))
                }
                (bulk_bar_view)
                table(
                    (head)
                    table_body(
                        for (key, cells) in &row_data {
                            let key_for_row = key.clone();
                            let key_for_action = key.clone();
                            table_row(
                                key: key_for_row,
                                for cell in cells {
                                    table_cell((cell.clone()))
                                }
                                if let Some(prefix) = &delete_prefix {
                                    table_cell(
                                        <form
                                            method="post"
                                            action=(format!("{}/{}/delete", prefix, key_for_action))
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
    pub async fn render_skeleton<'a>(&self, cx: &'a Cx) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let state = TableState::from_cx(cx);
        let path = topcoat::context::try_request_context::<http::request::Parts>(cx)
            .map(|parts| parts.uri.path().to_string())
            .unwrap_or_default();
        let head = self
            .render_thead(cx, &state, &path, self.delete_prefix.is_some())
            .await?;
        let column_count = self.columns.len();
        let with_delete = self.delete_prefix.is_some();
        let inner = view! {
            cx =>
            <div class="rounded-xl border border-border overflow-hidden">
                table(
                    (head)
                    table_body(
                        for i in 0..3 {
                            table_row(
                                key: i,
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
            </div>
        };
        Ok(if self.is_boundary {
            view! { cx => <div data-boundary="table">(inner)</div> }.boxed()
        } else {
            inner.boxed()
        })
    }

    /// Generate CSV for the given page (header + rows, RFC4180 escaped).
    pub fn to_csv(&self, page: &TablePage<M>) -> String
    where
        M: toasty::schema::Model,
    {
        fn escape_csv(s: &str) -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
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

    /// The GET search toolbar: submits `?q=` back to the current path,
    /// preserving the active sort and resetting pagination (a new search is a
    /// new result set). Renders a Clear link while a search is active.
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
                    ],
                )
            })
            .or_else(|| {
                filters_hidden
                    .as_ref()
                    .map(|f| build_url(path, &[("filters", Some(f.as_str()))]))
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
                ui_input(
                    attrs: attributes! {
                        type="search"
                        name="q"
                        value=(q_display)
                        placeholder="Search…"
                        aria-label="Search table"
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
        let clear_url = if !state.filters.is_empty() {
            Some(build_url(
                path,
                &[
                    ("q", state.search.as_deref()),
                    ("sort", state.sort.as_ref().map(|s| s.column.as_str())),
                    ("dir", dir_hidden),
                ],
            ))
        } else {
            None
        };
        Ok(view! {
            cx =>
            <form
                method="get"
                action=(action)
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
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
                ui_input(
                    attrs: attributes! {
                        type="text"
                        name="filters"
                        value=(filters_display)
                        placeholder="filters e.g. status:published"
                        aria-label="Filter table"
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
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let mut colspan = self.columns.len();
        if with_delete {
            colspan += 1;
        }
        let clear_url = if state.search.is_some() {
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
        let message = match &state.search {
            Some(term) => format!("No results for \u{201c}{term}\u{201d}"),
            None => "No records yet".to_string(),
        };
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
                                    "Clear search"
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
        let preserve: Vec<(&str, Option<&str>)> = vec![
            ("q", state.search.as_deref()),
            ("sort", state.sort.as_ref().map(|s| s.column.as_str())),
            ("dir", dir),
            ("filters", filters_param.as_deref()),
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
                let href = build_url(
                    path,
                    &[
                        ("q", state.search.as_deref()),
                        ("sort", Some(col.name())),
                        ("dir", Some(if next_desc { "desc" } else { "asc" })),
                        ("filters", state.filters_param().as_deref()),
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

#[derive(Debug, Default, Deserialize)]
struct TableQuery {
    q: Option<String>,
    sort: Option<String>,
    dir: Option<String>,
    after: Option<String>,
    before: Option<String>,
    filters: Option<String>,
    group_by: Option<String>,
}

impl TableState {
    /// Parse the state from the request in `cx`.
    ///
    /// A malformed query string parses as empty state rather than failing the
    /// request — a garbage `?q=` filters to nothing, and cursor errors surface
    /// later, at decode time, where they are precise. Renders without a
    /// request context (e.g. unit tests) get neutral state instead of a panic.
    pub fn from_cx(cx: &Cx) -> Self {
        if topcoat::context::try_request_context::<http::request::Parts>(cx).is_none() {
            return Self::default();
        }
        let parsed: TableQuery = topcoat::router::parse_query_params(cx).unwrap_or_default();
        let search = parsed
            .q
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let sort = parsed
            .sort
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|column| Sort {
                column: column.to_string(),
                descending: parsed.dir.as_deref() == Some("desc"),
            });
        let non_empty = |v: Option<String>| {
            v.filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_string())
        };
        let filters = parsed
            .filters
            .as_deref()
            .map(parse_filters_param)
            .unwrap_or_default();
        Self {
            search,
            sort,
            after: non_empty(parsed.after),
            before: non_empty(parsed.before),
            filters,
            group_by: non_empty(parsed.group_by),
        }
    }

    /// Serialized `filters` for URL (`key:value,key2:value2`), or `None` when empty.
    pub fn filters_param(&self) -> Option<String> {
        if self.filters.is_empty() {
            None
        } else {
            let mut pairs: Vec<String> = self
                .filters
                .iter()
                .map(|(k, v)| format!("{}:{}", k, v))
                .collect();
            pairs.sort();
            Some(pairs.join(","))
        }
    }
}

/// Parse `filters` query param: `key:value,key2:value2` (trimmed, blank ignored).
fn parse_filters_param(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            if !k.is_empty() && !v.is_empty() {
                map.insert(k, v);
            }
        }
    }
    map
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

/// Build `path?k=v&…` from ordered optional parameters, skipping `None`.
fn build_url(path: &str, params: &[(&str, Option<&str>)]) -> String {
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
}

impl Clone for NavigationItem {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            url: self.url.clone(),
            href_check: self.href_check.clone(),
        }
    }
}

impl std::fmt::Debug for NavigationItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationItem")
            .field("label", &self.label)
            .field("url", &self.url)
            .field("href_check", &self.href_check.is_some())
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
    /// `Policy::can_create` before calling this. The default implementation
    /// returns an error; resources should override to perform the actual
    /// `toasty::create!` (or `Insert`) inside a transaction.
    fn create_record(
        _cx: &Cx,
        _values: HashMap<String, String>,
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

    /// Update an existing record identified by `id` (the string form of its
    /// primary key, as produced by `Table::id`) from form values.
    fn update_record(
        _cx: &Cx,
        _id: String,
        _values: HashMap<String, String>,
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

    /// Delete a record by its string id.
    fn delete_record(_cx: &Cx, _id: String) -> impl std::future::Future<Output = Result<()>> + Send
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

    /// Bulk-delete records by their string ids. Default is per-row `delete_record`
    /// in a transaction; override for efficiency if needed.
    fn bulk_delete_records(
        _cx: &Cx,
        _ids: Vec<String>,
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
        };
        let showcase = NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
            href_check: None,
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
}
