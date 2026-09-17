//! `Resource` — maps one Toasty [`Model`] to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, and navigation. See
//! `CONTEXT.md` and ADR-0002.

use std::marker::PhantomData;
use std::sync::Arc;

use std::collections::HashMap;

use argentum_ui::{
    ButtonSize, ButtonVariant, alert_dialog, button, button_variants, dialog_content,
    dialog_description, dialog_footer, dialog_header, dialog_title, icons, input as ui_input,
    pagination, pagination_content, pagination_item, pagination_next, pagination_previous, table,
    table_body, table_cell, table_head, table_header, table_row,
};
use toasty::stmt::{Expr, List, OrderByExpr};
use topcoat::context::Cx;
use topcoat::icon::icon;
use topcoat::router::{Href, HrefParams, HrefQueries, HrefTarget};
use topcoat::runtime::{Event, Signal};
use topcoat::{Result, view::*};

use crate::schema::{FieldLens, Schema, capitalize, lens_field_name_and_label};

/// The live table's browser state (GH #151).
///
/// The page owns these signals and hands their handles to the `table_search`
/// shard through [`Table::render_live_with_state`]; each tracked read inside
/// the shard becomes a `dep` marker the browser watches, so writing any signal
/// re-renders the grid in place — no navigation, no scroll jump. Sort links,
/// the pager, the filter transport, and the clear links rendered by the table
/// write them.
///
/// `q`/`filters`/`sort`/`dir` reset the cursors when they change; `after` and
/// `before` page within the current result set. All values are untrusted by
/// the time the shard reads them back (the client owns the signal).
#[derive(Clone)]
pub struct TableSignals {
    /// `?q=` — the prefix search term.
    pub q: Signal<String>,
    /// `?filters=` — the composed `key:value,key2:value2` transport.
    pub filters: Signal<String>,
    /// `?sort=` — the active sort column name (`""` = the table default).
    pub sort: Signal<String>,
    /// `?dir=` — `asc`/`desc` for [`Self::sort`].
    pub dir: Signal<String>,
    /// `?after=` — the forward cursor.
    pub after: Signal<String>,
    /// `?before=` — the backward cursor.
    pub before: Signal<String>,
}

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
    interactive: bool,
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
            .field("interactive", &self.interactive)
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
            interactive: true,
            _marker: PhantomData,
        }
    }

    /// Render the list as a static preview: no search toolbar, no sort links,
    /// no pager links — just labels and rows.
    ///
    /// Demo pages that show a `Table` for its declaration (`searchable()` /
    /// `sortable()`) rather than its behavior use this, so a click cannot
    /// promise an interaction the page does not honor (GH #151: the showcase
    /// demos used to navigate to query strings the page ignored). Real
    /// resource lists keep the default.
    pub fn interactive(mut self, enabled: bool) -> Self {
        self.interactive = enabled;
        self
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
    ///
    /// Panics on duplicate [`Column::name`] (GH #156): sort resolution is
    /// first-sortable-`name()`-match, so duplicate sortable names would
    /// silently misresolve `?sort=`. The guard covers computed names too
    /// (`TextColumn::computed("Status", ..)` derives `name = "status"`) for
    /// namespace consistency and future-proofing. Same fail-loud policy as
    /// the GH #101 searchable/sortable panics and the Schema GH #100 guard.
    pub fn columns(mut self, cols: impl IntoColumns<M>) -> Self
    where
        M: toasty::schema::Model,
    {
        let cols = cols.into_columns();
        let mut seen = std::collections::HashSet::with_capacity(cols.len());
        for c in &cols {
            let name = c.name();
            assert!(
                seen.insert(name),
                "duplicate column name '{name}': each Table column needs a distinct name (GH #156)"
            );
        }
        self.columns = cols;
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
        for segment in &state.malformed_filters {
            out.push((segment.clone(), "malformed: expected key:value".to_string()));
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

    /// Normalize `state.group_by` against the declared grouping (GH #92, GH
    /// #153): an unknown `?group_by=` value renders no group headers and is
    /// dropped from every link instead of round-tripping.
    ///
    /// Each render seam normalizes at its own entry (`render_with_state`,
    /// `render_live_with_state`, `render_delete_dialog`,
    /// `render_live_search_bar`, `render_live_invocation`, `render_skeleton`,
    /// the shard handler, and the panel retry closures), so downstream links
    /// read the pre-normalized `state.group_by` field directly.
    pub(crate) fn normalize_state(&self, state: &TableState) -> TableState {
        let mut out = state.clone();
        if self.group_by.as_ref().map(|def| def.name.as_str()) != out.group_by.as_deref() {
            out.group_by = None;
        }
        out
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
    /// `searchable()`, so the toolbar and the query stay in step.
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
        let state = self.normalize_state(state);
        self.render_inner(cx, page, &state, path, None).await
    }

    /// Render the interactive grid for a live table (GH #151): the same
    /// presentation as [`Self::render_with_state`], with the sort links, the
    /// pager, the filter transport, and the empty-state clear links bound to
    /// `signals` — each interaction writes a signal and the browser morphs the
    /// shard's new output in place, without a navigation or a scroll jump.
    /// Every bound control keeps its real `href`/form, so a page without JS
    /// still navigates as before.
    pub async fn render_live_with_state<'a>(
        &self,
        cx: &'a Cx,
        page: TablePage<M>,
        state: &TableState,
        path: &str,
        signals: TableSignals,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        let state = self.normalize_state(state);
        self.render_inner(cx, page, &state, path, Some(signals))
            .await
    }

    /// Resolve and execute this table's query for `state` — search, filters,
    /// ordering, and cursor pagination — and return the rows.
    ///
    /// The loader half of the live-table seam (GH #154 §2): a page that owns
    /// its own table (the showcase demos) can hand its shard a query and this
    /// hook applies the same declaration pipeline `panel::load_table_page`
    /// applies to `Resource::query`, so a page-level shard does not
    /// reimplement filtering, ordering, or cursor validation.
    pub async fn load(
        &self,
        cx: &Cx,
        mut query: toasty::stmt::Query<List<M>>,
        state: &TableState,
    ) -> Result<TablePage<M>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        if self.page_size == Some(0) {
            return Err(std::io::Error::other(
                "Table::load: paginate requires per_page > 0 (GH #96)",
            )
            .into());
        }
        if let Some(term) = &state.search
            && let Some(expr) = self.search_expr(term)
        {
            query = query.filter(expr);
        }
        if let Some(expr) = self.filter_expr(state) {
            query = query.filter(expr);
        }
        for ord in self.order_bys_for_state(state) {
            query = query.order_by(ord);
        }
        let mut db = crate::db::db(cx);
        match self.page_size {
            Some(per_page) => {
                // Keep a cursor-free copy of the filtered+ordered query for
                // cursor validation. Toasty's `Page` sets `next_cursor` when
                // `len == page_size` and `prev_cursor` when `has_previous_page`,
                // which leaves phantom cursors when the page sits exactly at a
                // boundary (e.g. a `before` fetch that lands on the first page
                // returns `len == page_size` so `prev_cursor` is set even though
                // `before(prev_cursor)` is empty). Validate such cursors with a
                // cheap `LIMIT 1` probe and hide phantoms.
                let base_query = query.clone();
                let mut paginated = toasty::stmt::Paginate::new(query, per_page);
                // Toasty cursor pagination takes exactly one cursor (GH #155):
                // a URL carrying both `?after=` and `?before=` must fail loudly
                // instead of silently preferring `after` (the GH #93 fail-open
                // family). The `CursorDecodeError` marker gives the failure the
                // drop-pagination retry contract (GH #110).
                if state.after.is_some() && state.before.is_some() {
                    return Err(crate::cursor::CursorDecodeError::conflicting_cursors());
                }
                if let Some(cursor) = &state.after {
                    paginated = paginated.after(crate::cursor::decode(cursor)?);
                } else if let Some(cursor) = &state.before {
                    paginated = paginated.before(crate::cursor::decode(cursor)?);
                }
                let loaded = paginated
                    .exec(&mut db)
                    .await
                    .map_err(topcoat::Error::from)?;
                let mut page = TablePage::from_toasty_page(loaded)?;
                // Only probe for phantom cursors when the page is full
                // (`len == per_page`); a short page cannot have a next page
                // and probing would be a wasted round-trip (GH #75).
                if page.rows.len() == per_page {
                    if let Some(cursor) = page.next_cursor.clone() {
                        let probe = toasty::stmt::Paginate::new(base_query.clone(), 1)
                            .after(crate::cursor::decode(&cursor)?)
                            .exec(&mut db)
                            .await
                            .map_err(topcoat::Error::from)?;
                        if probe.items.is_empty() {
                            page.next_cursor = None;
                        }
                    }
                    if let Some(cursor) = page.prev_cursor.clone() {
                        let probe = toasty::stmt::Paginate::new(base_query.clone(), 1)
                            .before(crate::cursor::decode(&cursor)?)
                            .exec(&mut db)
                            .await
                            .map_err(topcoat::Error::from)?;
                        if probe.items.is_empty() {
                            page.prev_cursor = None;
                        }
                    }
                } else {
                    // Short page → no next, keep prev as-is (has_previous already correct).
                    page.next_cursor = None;
                }
                Ok(page)
            }
            None => {
                let rows: Vec<M> = query.exec(&mut db).await.map_err(topcoat::Error::from)?;
                Ok(rows.into())
            }
        }
    }

    async fn render_inner<'a>(
        &self,
        cx: &'a Cx,
        page: TablePage<M>,
        state: &TableState,
        path: &str,
        signals: Option<TableSignals>,
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
            .render_thead(
                cx,
                state,
                path,
                delete_prefix.is_some(),
                with_bulk,
                signals.as_ref(),
            )
            .await?;
        let show_search = self.search_enabled();
        let search_bar = if show_search {
            Some(self.render_search_bar(cx, state, path).await?)
        } else {
            None
        };
        let show_filters = !self.filters.is_empty();
        let filter_bar = if show_filters {
            Some(
                self.render_filter_bar(cx, state, path, signals.as_ref())
                    .await?,
            )
        } else {
            None
        };
        let bulk_bar_view: BoxView<'_> = if with_bulk {
            let bulk_action = format!("{}/bulk-delete", self.delete_prefix.clone().unwrap());
            let csrf = crate::csrf::current_token(cx);
            // No visible `ids` field (GH #151): the transport is hidden and
            // fed by the row checkboxes (`bulk.js`), and the destructive
            // submit ships disabled so an empty submit cannot be produced
            // from the UI — the script enables it once a row is checked.
            view! {
                cx =>
                <form
                    method="post"
                    action=(bulk_action)
                    class="flex gap-2 p-3 border-b border-border"
                    data-bulk-form=""
                >
                    <input type="hidden" name="csrf_token" value=(csrf)>
                    <input type="hidden" name="ids" value="" data-bulk-ids="">
                    button(
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Md,
                        attrs: attributes! { type="submit" disabled="" data-bulk-submit="" },
                        "Bulk Delete"
                    )
                </form>
            }
            .boxed()
        } else {
            view! { cx => <span></span> }.boxed()
        };
        let pager = self
            .render_pager(cx, state, path, &page, signals.as_ref())
            .await?;
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
                // No false tail: when other filters still apply, "unfiltered"
                // would be a lie (GH #148 — a malformed segment can ride
                // alongside valid ones).
                let consequence = if state.filters.is_empty() {
                    "showing unfiltered results"
                } else {
                    "other filter(s) still apply"
                };
                let text = format!("Ignored filter(s): {detail} — {consequence}.");
                let clear = state.without_filters(path);
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
        //
        // The per-row delete URL (GH #151) opens the confirmation dialog on
        // the list page (`?delete=<key>`) instead of posting straight away.
        let row_data: Vec<(String, Vec<String>, Option<String>)> = page
            .rows
            .iter()
            .map(|row| {
                let key = row_key(row);
                let cells: Vec<String> = self
                    .columns
                    .iter()
                    .map(|col| col.render_cell(row))
                    .collect();
                let open_url = delete_prefix
                    .is_some()
                    .then(|| state.with_delete_dialog(path, &key));
                (key, cells, open_url)
            })
            .collect();
        // Row keys must be injective within a page (GH #96): duplicates corrupt
        // keyed diffs and bulk selection (two rows, one checkbox value).
        debug_assert!(
            {
                let mut seen = std::collections::HashSet::new();
                row_data.iter().all(|(k, _, _)| seen.insert(k.clone()))
            },
            "duplicate Table::id keys in one page: Table::id must be injective"
        );
        // The confirmation dialog lives with the delete chrome (GH #151); the
        // live-search page renders it outside the shard region instead.
        let delete_dialog = self.render_delete_dialog(cx, state, path).await?;

        if page.rows.is_empty() {
            let empty_cell = self
                .render_empty_cell(
                    cx,
                    state,
                    path,
                    delete_prefix.is_some(),
                    with_bulk,
                    signals.as_ref(),
                )
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
                    if let Some(warning) = filter_warning {
                        (warning)
                    }
                    table(
                        (head)
                        (empty_cell)
                    )
                    if let Some(dialog) = delete_dialog {
                        (dialog)
                    }
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
                        #[key(key.as_str())]
                        for (key, cells, open_url) in &row_data {
                            let key_for_row = key.clone();
                            let key_for_select = key.clone();
                            let open_for_row = open_url.clone();
                            let row_dom_id = row_dom_id(&key_for_row);
                            table_row(
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
                                if let Some(url) = open_for_row {
                                    table_cell(
                                        <a
                                            href=(url)
                                            class=(button_variants(
                                                ButtonVariant::Destructive,
                                                ButtonSize::Md,
                                            ))
                                        >
                                            "Delete"
                                        </a>
                                    )
                                }
                            )
                        }
                    )
                )
                for p in pager {
                    (p)
                }
                if let Some(dialog) = delete_dialog {
                    (dialog)
                }
            </div>
        };
        Ok(if is_boundary {
            view! { cx => <div data-boundary="table">(inner)</div> }.boxed()
        } else {
            inner.boxed()
        })
    }

    /// The row-delete confirmation dialog (GH #151), rendered when the URL
    /// asks for one and [`Self::with_delete`] wired the delete route.
    ///
    /// `?delete=<row key>` opens the alert dialog on the list page; the
    /// dialog's form POSTs to `{prefix}/{key}/delete` with `confirm=1` — the
    /// same two-step route as the old confirmation page, now with a
    /// Destructive confirm. Cancel is a link back to the list state without
    /// `?delete=`; `dialog.js` adds Escape/backdrop dismissal and mirrors it
    /// as `?open=false` ([`TableState::open`]), so a reload stays closed.
    ///
    /// [`Self::render_with_state`] renders it with the grid; the live-search
    /// page (`panel::resource_list_live`) calls this separately because the
    /// shard swaps the grid per keystroke and must not carry dialog state.
    pub async fn render_delete_dialog<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
    ) -> Result<Option<BoxView<'a>>> {
        let state = self.normalize_state(state);
        let Some(prefix) = self.delete_prefix.as_deref() else {
            return Ok(None);
        };
        let Some(key) = state.delete.as_deref() else {
            return Ok(None);
        };
        if state.open == Some(false) {
            return Ok(None);
        }
        let action = format!("{}/{}/delete", prefix, encode_path_segment(key));
        let csrf = crate::csrf::current_token(cx);
        let cancel_url = state.list_url(path);
        Ok(Some(
            view! {
                cx =>
                alert_dialog(
                    open: true,
                    attrs: attributes! {
                        aria-labelledby="delete-dialog-title"
                        aria-describedby="delete-dialog-description"
                        data-dialog-open-param="open"
                    },
                    dialog_content(
                        dialog_header(
                            dialog_title(
                                attrs: attributes! { id="delete-dialog-title" },
                                "Delete this record?"
                            )
                            dialog_description(
                                attrs: attributes! { id="delete-dialog-description" },
                                "This action cannot be undone."
                            )
                        )
                        dialog_footer(
                            <form method="post" action=(action) class="contents">
                                <a
                                    href=(cancel_url)
                                    data-dialog-close=""
                                    class=(button_variants(
                                        ButtonVariant::Outline,
                                        ButtonSize::Md,
                                    ))
                                >
                                    "Cancel"
                                </a>
                                <input type="hidden" name="confirm" value="1">
                                <input type="hidden" name="csrf_token" value=(csrf)>
                                button(
                                    variant: ButtonVariant::Destructive,
                                    size: ButtonSize::Md,
                                    attrs: attributes! { type="submit" },
                                    "Delete"
                                )
                            </form>
                        )
                    )
                )
            }
            .boxed(),
        ))
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
        // Same normalization as the grid seams (GH #153): the placeholder
        // header links must not echo an unknown `?group_by=`.
        let state = self.normalize_state(&state);
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
                None,
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
                        #[key(i)]
                        for i in 0..3 {
                            table_row(
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
            // The busy state rides on the morph boundary (GH #160) so assistive
            // tech sees the live region, not just the swapped root below it.
            view! { cx => <div data-boundary="table" aria-busy="true">(inner)</div> }.boxed()
        } else {
            inner.boxed()
        })
    }

    /// Generate CSV for the given page (header + rows, RFC4180 escaped).
    ///
    /// Formula cells are defused per OWASP (a leading `'` is prepended when
    /// the first non-whitespace/control character is `=`, `+`, `-`, `@`, `|`,
    /// or `%`) so a stored value like `=1+1` — including CR/LF- or tab-led
    /// variants, which spreadsheets treat as formulas even when the payload
    /// does not start the raw cell (GH #145) — opens as text, not a live
    /// spreadsheet formula. The page passed in is buffered as one `String`;
    /// the export handler caps the filtered query (GH #94) so callers cannot
    /// buffer an unbounded table.
    pub fn to_csv(&self, page: &TablePage<M>) -> String
    where
        M: toasty::schema::Model,
    {
        fn defuse_formula(s: &str) -> String {
            // Spreadsheets run formulas led by CR/LF/tab too (OWASP CSV
            // injection): the dangerous payload can start mid-cell after
            // leading whitespace, controls, or zero-width format characters
            // (BOM/ZWSP are neither whitespace nor control), so the
            // first-char test skips them. The `'` lands on the original
            // cell, before the payload.
            let trimmed = s.trim_start_matches(|c: char| {
                c.is_whitespace() || c.is_control() || matches!(c, '\u{FEFF}' | '\u{200B}')
            });
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
    /// or auto — at least one `searchable()` column. A non-interactive
    /// preview never renders it (GH #151).
    pub(crate) fn search_enabled(&self) -> bool
    where
        M: toasty::schema::Model,
    {
        self.interactive
            && self
                .search_ui
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
        // Pre-normalized by the render seams (GH #153): `state.group_by` is
        // the declared name or `None`, never an unknown value (GH #92).
        let group_hidden = state.group_by.clone();
        // Clear only renders when something survives the search term; every
        // branch below projects the same URL, so one intent serves all three.
        let clear_url =
            (state.sort.is_some() || filters_hidden.is_some() || group_hidden.is_some())
                .then(|| state.without_search(path));
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
    ///
    /// Typing writes `q` and clears the cursors (a new term is a new result
    /// set); the shard re-renders in place (GH #151).
    ///
    /// Public so a page owning its own signals can render the same toolbar
    /// above its own shard (the showcase demos, GH #154 §2); resource lists
    /// reach it through `panel::resource_list_live`.
    pub async fn render_live_search_bar<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        signals: &TableSignals,
    ) -> Result<BoxView<'a>> {
        // Called directly with raw state (panel live page, showcase demos):
        // normalize for the `<noscript>` fallback links (GH #153).
        let state = self.normalize_state(state);
        let fallback = self.render_search_bar(cx, &state, path).await?;
        let q = signals.q.clone();
        let (after, before) = (signals.after.clone(), signals.before.clone());
        Ok(view! {
            cx =>
            <div
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
                data-live-search=""
            >
                <input
                    :value=$(q.get())
                    @input=$(|e: Event| {
                        q.set(e.target.value);
                        after.set("".to_owned());
                        before.set("".to_owned());
                    })
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
    /// region (GH #104). The signal handles travel as arguments; every
    /// tracked read inside the shard becomes a `dep` marker the browser
    /// watches, so sort/filter/pager/search changes re-render the grid in
    /// place (GH #151).
    pub(crate) async fn render_live_invocation<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        signals: TableSignals,
    ) -> Result<BoxView<'a>> {
        use crate::panel::table_search;

        let state = self.normalize_state(state);
        let live_path = path.to_string();
        let live_group = state.group_by.clone().unwrap_or_default();
        let TableSignals {
            q,
            filters,
            sort,
            dir,
            after,
            before,
        } = signals;
        Ok(view! {
            cx =>
            table_search(
                path: $(live_path.clone()),
                q: $(q),
                filters: $(filters),
                sort: $(sort),
                dir: $(dir),
                after: $(after),
                before: $(before),
                group_by: $(live_group.clone())
            )
        }
        .boxed())
    }

    /// The typed filter bar. For live tables (`signals`) the hidden `filters`
    /// transport is bound to the `filters` signal and `filters.js` dispatches
    /// a `change` into it instead of submitting, so the shard re-renders the
    /// grid in place; the GET form stays as the no-JS fallback and `href`s
    /// remain real.
    async fn render_filter_bar<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        signals: Option<&TableSignals>,
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
        let group_hidden = state.group_by.clone();
        let clear_url = if !state.filters.is_empty() {
            Some(state.without_filters(path))
        } else {
            None
        };
        // One typed control per declared filter (GH #74). Controls carry only
        // `data-filter-name` (no `name`, so they never submit on their own);
        // `filters.js` composes them into the hidden `filters` transport and
        // submits on change (GH #151), rewriting it even when every control is
        // "All" so the stale value can never be resubmitted. The free-text
        // input and Apply button survive only inside `<noscript>` as the
        // no-JS fallback.
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
        let form_attrs = attributes! {
            cx =>
            method="get"
            action=(action)
            class="flex flex-wrap items-center gap-2 border-b border-border p-3"
            data-filters-form=""
            if signals.is_some() {
                data-filters-live=""
            }
        };
        // Live tables bind the transport to the `filters` signal: `filters.js`
        // composes and dispatches, the shard re-renders in place. Static
        // tables keep the server-rendered value the GET form submits.
        let transport_attrs = if let Some(signals) = signals {
            let (filters, after, before) = (
                signals.filters.clone(),
                signals.after.clone(),
                signals.before.clone(),
            );
            attributes! {
                cx =>
                name="filters"
                :value=$(filters.get())
                @change=$(|e: Event| {
                    filters.set(e.target.value);
                    after.set("".to_owned());
                    before.set("".to_owned());
                })
                data-filters-transport=""
            }
        } else {
            attributes! {
                cx =>
                name="filters"
                value=(filters_display.clone())
                data-filters-transport=""
            }
        };
        let clear_link: Option<BoxView<'a>> = clear_url.map(|url| {
            let attrs = match signals {
                Some(signals) => {
                    let (filters, after, before) = (
                        signals.filters.clone(),
                        signals.after.clone(),
                        signals.before.clone(),
                    );
                    attributes! {
                        cx =>
                        href=(url.clone())
                        @click=$(|e: Event| {
                            e.prevent_default();
                            filters.set("".to_owned());
                            after.set("".to_owned());
                            before.set("".to_owned());
                        })
                    }
                }
                None => attributes! { cx => href=(url) },
            };
            view! {
                cx =>
                <a class="text-sm text-muted-foreground hover:text-foreground" (attrs)>
                    "Clear filters"
                </a>
            }
            .boxed()
        });
        Ok(view! {
            cx =>
            <form (form_attrs)>
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
                <noscript>
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
                </noscript>
                <input type="hidden" (transport_attrs)>
                if let Some(link) = clear_link {
                    (link)
                }
            </form>
        }
        .boxed())
    }

    /// The zero-rows cell — one honest message, not two: "no records yet"
    /// when unfiltered, "no results" with a Clear link when a search is
    /// active. The dead Create button is gone (create pages are not wired
    /// yet). Wrapped in a single cell spanning the table so it sits inside
    /// the grid. For live tables (`signals`) the clear/back links write the
    /// signals instead of navigating; `href` stays the fallback.
    async fn render_empty_cell<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        with_delete: bool,
        with_bulk: bool,
        signals: Option<&TableSignals>,
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
        // Clear only the dimension the link names and keep the rest of the
        // state (GH #93 follow-up): the old link rebuilt the URL from `sort`
        // alone — dropping `group_by` — and cleared the filters too under a
        // "Clear search" label when both a search and filters were active.
        let clear_url = filtered.then(|| {
            if state.search.is_some() {
                state.without_search(path)
            } else {
                state.without_filters(path)
            }
        });
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
        let first_page_url =
            (state.after.is_some() || state.before.is_some()).then(|| state.without_cursor(path));
        // Live links write the signals in place (keeping the state the link
        // does not name); `href` stays the no-JS fallback.
        let clear_link: Option<BoxView<'a>> = clear_url.map(|url| {
            let attrs = match signals {
                Some(signals) => {
                    let (q, filters, after, before) = (
                        signals.q.clone(),
                        signals.filters.clone(),
                        signals.after.clone(),
                        signals.before.clone(),
                    );
                    let clearing_search = state.search.is_some();
                    attributes! {
                        cx =>
                        href=(url)
                        @click=$(|e: Event| {
                            e.prevent_default();
                            if clearing_search {
                                q.set("".to_owned());
                            } else {
                                filters.set("".to_owned());
                            }
                            after.set("".to_owned());
                            before.set("".to_owned());
                        })
                    }
                }
                None => attributes! { cx => href=(url) },
            };
            view! {
                cx =>
                <a class="text-sm text-primary hover:underline" (attrs)>
                    (clear_label)
                </a>
            }
            .boxed()
        });
        let first_page_link: Option<BoxView<'a>> = first_page_url.map(|url| {
            let attrs = match signals {
                Some(signals) => {
                    let (after, before) = (signals.after.clone(), signals.before.clone());
                    attributes! {
                        cx =>
                        href=(url)
                        @click=$(|e: Event| {
                            e.prevent_default();
                            after.set("".to_owned());
                            before.set("".to_owned());
                        })
                    }
                }
                None => attributes! { cx => href=(url) },
            };
            view! {
                cx =>
                <a class="text-sm text-primary hover:underline" (attrs)>
                    "Back to first page"
                </a>
            }
            .boxed()
        });
        Ok(view! {
            cx =>
            table_body(
                table_row(
                    table_cell(
                        attrs: attributes! { colspan=(colspan) class="px-6 py-16 text-center" },
                        <div class="flex flex-col items-center gap-4">
                            <p class="text-sm text-muted-foreground">(message)</p>
                            if let Some(link) = clear_link {
                                (link)
                            }
                            if let Some(link) = first_page_link {
                                (link)
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
    ///
    /// With `signals` (a live table) each link also writes its cursor signal
    /// and clears the opposite one; `href` stays the no-JS fallback.
    async fn render_pager<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        page: &TablePage<M>,
        signals: Option<&TableSignals>,
    ) -> Result<Vec<BoxView<'a>>> {
        if self.page_size.is_none() || !self.interactive {
            return Ok(Vec::new());
        }
        // Cursors only carry ordering values; the loader re-applies search and
        // sort, so the links must carry that state along.
        let next_href = page
            .next_cursor
            .as_ref()
            .map(|cursor| state.with_after(path, cursor));
        let prev_href = page
            .prev_cursor
            .as_ref()
            .map(|cursor| state.with_before(path, cursor));
        if prev_href.is_none() && next_href.is_none() {
            return Ok(Vec::new());
        }
        let prev_item: Option<BoxView<'a>> = prev_href.map(|href| {
            let attrs = match (signals, page.prev_cursor.as_deref()) {
                (Some(signals), Some(cursor)) => {
                    let (after, before) = (signals.after.clone(), signals.before.clone());
                    let cursor = cursor.to_owned();
                    attributes! {
                        cx =>
                        href=(href.clone())
                        @click=$(|e: Event| {
                            e.prevent_default();
                            before.set(cursor.clone());
                            after.set("".to_owned());
                        })
                    }
                }
                _ => attributes! { cx => href=(href) },
            };
            view! { cx => pagination_item(pagination_previous(attrs: attrs)) }.boxed()
        });
        let next_item: Option<BoxView<'a>> = next_href.map(|href| {
            let attrs = match (signals, page.next_cursor.as_deref()) {
                (Some(signals), Some(cursor)) => {
                    let (after, before) = (signals.after.clone(), signals.before.clone());
                    let cursor = cursor.to_owned();
                    attributes! {
                        cx =>
                        href=(href.clone())
                        @click=$(|e: Event| {
                            e.prevent_default();
                            after.set(cursor.clone());
                            before.set("".to_owned());
                        })
                    }
                }
                _ => attributes! { cx => href=(href) },
            };
            view! { cx => pagination_item(pagination_next(attrs: attrs)) }.boxed()
        });
        let pager = view! {
            cx =>
            <div class="border-t border-border p-3">
                pagination(
                    pagination_content(
                        if let Some(item) = prev_item {
                            (item)
                        }
                        if let Some(item) = next_item {
                            (item)
                        }
                    )
                )
            </div>
        };
        Ok(vec![pager.boxed()])
    }

    /// The shared column-header row — the single source of the `<thead>`
    /// markup: labels and **links** on sortable columns that toggle
    /// `?sort=`/`?dir=` (a Lucide arrow with `aria-sort` when active,
    /// `arrow-up-down` when inactive). Every render branch (skeleton / empty
    /// / rows) composes it, so an a11y or styling change happens once.
    ///
    /// With `signals` (a live table) the link also writes the sort signals and
    /// clears the cursors; its `href` stays the no-JS fallback.
    async fn render_thead<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        with_delete: bool,
        with_bulk: bool,
        signals: Option<&TableSignals>,
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
            // A static preview renders plain labels: no link to an interaction
            // the page does not honor (GH #151).
            let sortable = self.interactive && col.is_sortable();
            let (head_class, aria_sort, header) = if sortable {
                let (aria, sort_icon, next_desc) = match active {
                    Some(s) if s.column == col.name() => (
                        if s.descending {
                            "descending"
                        } else {
                            "ascending"
                        },
                        if s.descending {
                            icons::ARROW_DOWN
                        } else {
                            icons::ARROW_UP
                        },
                        // toggling the active column flips the direction
                        !s.descending,
                    ),
                    _ => ("none", icons::ARROW_UP_DOWN, false),
                };
                let href = state.sorted_by(path, col.name(), next_desc);
                let aria_label = format!(
                    "Sort by {} {}",
                    label,
                    if next_desc { "descending" } else { "ascending" }
                );
                let link_attrs = if let Some(signals) = signals {
                    let (sort, dir, after, before) = (
                        signals.sort.clone(),
                        signals.dir.clone(),
                        signals.after.clone(),
                        signals.before.clone(),
                    );
                    let column = col.name().to_string();
                    let next_dir = if next_desc { "desc" } else { "asc" }.to_owned();
                    attributes! {
                        cx =>
                        href=(href)
                        aria-label=(aria_label)
                        @click=$(|e: Event| {
                            e.prevent_default();
                            sort.set(column.clone());
                            dir.set(next_dir.clone());
                            after.set("".to_owned());
                            before.set("".to_owned());
                        })
                    }
                } else {
                    attributes! { cx => href=(href) aria-label=(aria_label) }
                };
                (
                    "cursor-pointer hover:bg-foreground/5",
                    Some(aria),
                    view! {
                        cx =>
                        <a
                            class="inline-flex items-center gap-1 hover:text-foreground"
                            (link_attrs)
                        >
                            (label.clone())
                            icon(
                                data: sort_icon,
                                attrs: attributes! { class="size-4 shrink-0 text-muted-foreground" }
                            )
                        </a>
                    }
                    .boxed(),
                )
            } else {
                ("", None, view! { cx => (label.clone()) }.boxed())
            };
            heads.push(
                view! {
                    cx =>
                    table_head(
                        attrs: attributes! { class=(head_class) aria-sort=(aria_sort) },
                        (header)
                    )
                }
                .boxed(),
            );
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
    /// `?q=` — trimmed and clamped to [`MAX_QUERY_TERM`] chars; `None` when
    /// absent or blank.
    pub search: Option<String>,
    /// `?sort=` + `?dir=` — `None` when absent or blank.
    pub sort: Option<Sort>,
    /// `?after=` — encoded forward cursor.
    pub after: Option<String>,
    /// `?before=` — encoded backward cursor.
    pub before: Option<String>,
    /// `?filters=` — `key:value,key2:value2` (comma-separated, colon-delimited).
    pub filters: HashMap<String, String>,
    /// `?filters=` segments that carry no `key:value` pair (GH #148): kept so
    /// [`Table::unapplied_filters`] can flag them (list banner, export 400)
    /// instead of silently dropping them, and so [`Self::filters_param`]
    /// round-trips them — a link built from this state keeps the warning
    /// until a valid `?filters=` replaces it.
    pub malformed_filters: Vec<String>,
    /// `?group_by=` — field name to group by (in-memory, `count` summarizer).
    pub group_by: Option<String>,
    /// `?delete=` — the row key whose delete confirmation dialog opens on the
    /// list page (GH #151). The dialog's confirmed POST re-enters the delete
    /// route; the parameter itself is never a write.
    pub delete: Option<String>,
    /// `?open=false` — set by `dialog.js` when Escape/backdrop dismisses the
    /// delete dialog, so the next render stays closed. Absent (or `true`)
    /// renders it open.
    pub open: Option<bool>,
}

/// Longest search term accepted (`?q=` and the shard's `q`, GH #148): bounded
/// echoed state, matching the live-search shard's clamp.
pub(crate) const MAX_QUERY_TERM: usize = 128;

/// Clamp a search term to [`MAX_QUERY_TERM`] chars (chars, not bytes, so a
/// multibyte term truncates on boundaries).
pub(crate) fn clamp_query_term(term: &str) -> String {
    term.trim().chars().take(MAX_QUERY_TERM).collect()
}

impl TableState {
    /// Parse the state from the request in `cx`.
    ///
    /// A blank or unknown query parses as neutral state rather than failing
    /// the request. Duplicate keys (`?filters=a&filters=b`) resolve to the
    /// first occurrence: the previous serde decode rejected duplicates, and
    /// swallowing that error as empty state silently dropped every filter —
    /// including export's fail-closed guard (GH #93). Cursor errors still
    /// surface later, at decode time, where they are precise — including the
    /// conflicting `after` + `before` pair, which fails at load time (GH #155).
    /// Renders without a request context (e.g. unit tests) get neutral state.
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
        let (filters, malformed_filters) = parse_filters_param(get("filters").unwrap_or_default());
        Self {
            search: get("q").map(clamp_query_term).filter(|t| !t.is_empty()),
            sort: non_empty(get("sort")).map(|column| Sort {
                column,
                descending: get("dir") == Some("desc"),
            }),
            after: non_empty(get("after")),
            before: non_empty(get("before")),
            filters,
            malformed_filters,
            group_by: non_empty(get("group_by")),
            // The delete dialog is opt-in through `?delete=`; `?open=false`
            // is the dismissal mirror `dialog.js` writes (GH #151). Any other
            // `open` value stays neutral (open).
            delete: non_empty(get("delete")),
            open: match get("open") {
                Some("false") => Some(false),
                Some("true") => Some(true),
                _ => None,
            },
        }
    }

    /// Serialized `filters` for URL (`key:value,key2:value2`), or `None` when empty.
    ///
    /// Keys/values escape `%`, `:`, `,` (`%25`/`%3A`/`%2C`, GH #93) so a
    /// free-text value like `a,b` round-trips instead of splitting.
    pub fn filters_param(&self) -> Option<String> {
        if self.filters.is_empty() && self.malformed_filters.is_empty() {
            return None;
        }
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
        // Malformed segments ride along verbatim (GH #148): they have no
        // colon to protect and re-enter `parse_filters_param` as malformed on
        // the next request, keeping the banner (and export's fail-closed 400)
        // alive across pagination.
        pairs.extend(self.malformed_filters.iter().cloned());
        Some(pairs.join(","))
    }

    /// URL projection: `TableState` owns the table's URL vocabulary (GH #153).
    /// Callers ask for a user intent, never a parameter list, so adding a
    /// parameter cannot silently drop it from half the links (GH #93).
    ///
    /// One private encoder ([`Self::project_url`]) holds the vocabulary in
    /// canonical order `q, sort, dir, filters, group_by, after, before`
    /// (`delete` appended by its intent). The parser is first-wins with unique
    /// keys, so order is semantically irrelevant.
    ///
    /// Expects `group_by` pre-normalized: render seams normalize through
    /// [`Table::normalize_state`], so the projection echoes `state.group_by`
    /// as-is. `open` is never emitted by any link; `delete` only by
    /// [`Self::with_delete_dialog`].
    ///
    /// Full state, including cursors; never `delete`/`open`. The streamed
    /// retry link for failures that keep their evidence (GH #98).
    pub(crate) fn list_url(&self, path: &str) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            self.search.as_deref(),
            self.sort_pair(),
            filters.as_deref(),
            self.group_by.as_deref(),
            self.after.as_deref(),
            self.before.as_deref(),
            None,
        )
    }

    /// Drops `q` (and the cursors + dialog of its result set); keeps the
    /// `filters` transport including malformed segments (GH #148).
    pub(crate) fn without_search(&self, path: &str) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            None,
            self.sort_pair(),
            filters.as_deref(),
            self.group_by.as_deref(),
            None,
            None,
            None,
        )
    }

    /// Drops `filters` and malformed segments (and the cursors + dialog of
    /// their result set); keeps the search term.
    pub(crate) fn without_filters(&self, path: &str) -> String {
        self.project_url(
            path,
            self.search.as_deref(),
            self.sort_pair(),
            None,
            self.group_by.as_deref(),
            None,
            None,
            None,
        )
    }

    /// Drops `after` and `before`; keeps everything else. Back-to-first-page
    /// and the cursor-failure retry link (GH #110).
    pub(crate) fn without_cursor(&self, path: &str) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            self.search.as_deref(),
            self.sort_pair(),
            filters.as_deref(),
            self.group_by.as_deref(),
            None,
            None,
            None,
        )
    }

    /// Full state + `after`, drops `before` and the dialog.
    pub(crate) fn with_after(&self, path: &str, token: &str) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            self.search.as_deref(),
            self.sort_pair(),
            filters.as_deref(),
            self.group_by.as_deref(),
            Some(token),
            None,
            None,
        )
    }

    /// Full state + `before`, drops `after` and the dialog.
    pub(crate) fn with_before(&self, path: &str, token: &str) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            self.search.as_deref(),
            self.sort_pair(),
            filters.as_deref(),
            self.group_by.as_deref(),
            None,
            Some(token),
            None,
        )
    }

    /// Replaces `sort`/`dir`, drops cursors and the dialog: a new ordering is
    /// a new result set.
    pub(crate) fn sorted_by(&self, path: &str, column: &str, descending: bool) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            self.search.as_deref(),
            Some((column, if descending { "desc" } else { "asc" })),
            filters.as_deref(),
            self.group_by.as_deref(),
            None,
            None,
            None,
        )
    }

    /// Full state including cursors + `delete=key`; never `open` (GH #151).
    pub(crate) fn with_delete_dialog(&self, path: &str, key: &str) -> String {
        let filters = self.filters_param();
        self.project_url(
            path,
            self.search.as_deref(),
            self.sort_pair(),
            filters.as_deref(),
            self.group_by.as_deref(),
            self.after.as_deref(),
            self.before.as_deref(),
            Some(key),
        )
    }

    /// `?sort=` column + `?dir=` value for the projection.
    fn sort_pair(&self) -> Option<(&str, &str)> {
        self.sort
            .as_ref()
            .map(|s| (s.column.as_str(), if s.descending { "desc" } else { "asc" }))
    }

    /// The one encoder: every table link's parameter vocabulary lives here.
    ///
    /// The exhaustive destructure fails compilation when a field is added to
    /// `TableState`, forcing the author to decide where it projects.
    #[allow(clippy::too_many_arguments)]
    fn project_url(
        &self,
        path: &str,
        search: Option<&str>,
        sort: Option<(&str, &str)>,
        filters: Option<&str>,
        group_by: Option<&str>,
        after: Option<&str>,
        before: Option<&str>,
        delete: Option<&str>,
    ) -> String {
        let TableState {
            search: _,
            sort: _,
            after: _,
            before: _,
            filters: _,
            malformed_filters: _,
            group_by: _,
            delete: _,
            open: _,
        } = self;
        build_url(
            path,
            &[
                ("q", search),
                ("sort", sort.map(|(column, _)| column)),
                ("dir", sort.map(|(_, dir)| dir)),
                ("filters", filters),
                ("group_by", group_by),
                ("after", after),
                ("before", before),
                ("delete", delete),
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
        let (filters, malformed_filters) = parse_filters_param(filters_param);
        Self {
            search,
            sort,
            after: None,
            before: None,
            filters,
            malformed_filters,
            group_by: non_empty(group_by),
            // Live search resets the delete dialog with pagination: a
            // keystroke is a new result set, and the panel renders the dialog
            // outside the shard region for live tables (GH #151).
            delete: None,
            open: None,
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
///
/// Segments that carry no `key:value` pair — colon-less (`foobar`), or an
/// empty key/value after decoding — are returned separately (GH #148): they
/// are flagged by [`Table::unapplied_filters`] (list banner, export 400)
/// instead of being silently dropped, and round-trip through
/// [`TableState::filters_param`] verbatim.
fn parse_filters_param(raw: &str) -> (HashMap<String, String>, Vec<String>) {
    let mut map = HashMap::new();
    let mut malformed = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Split on the first *unescaped* colon: `%3A` stays inside the key/value,
        // so a plain `split_once(':')` is correct on the encoded form.
        let Some((k_enc, v_enc)) = part.split_once(':') else {
            malformed.push(part.to_string());
            continue;
        };
        let k = decode_filter_component(k_enc.trim());
        let v = decode_filter_component(v_enc.trim());
        if k.is_empty() || v.is_empty() {
            malformed.push(part.to_string());
        } else {
            map.entry(k).or_insert(v);
        }
    }
    (map, malformed)
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
    /// semantics for string URLs: exact path match, or prefix match on a slash
    /// boundary — uniform for every item, resources and custom links alike
    /// (GH #39/#148: since resources mount at `{prefix}/{slug}`, no generated
    /// item points at the bare panel prefix, so the old root-exact special
    /// case is gone and the doc no longer promises one).
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
    ///
    /// Also gates relationship option loads (GH #108): a related resource
    /// that denies this cannot offer its records as options at all.
    fn can_view_any(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may view the given record.
    ///
    /// Checked on the edit page (GET), the edit POST (which requires both
    /// `can_view` and `can_update`, GH #86), per row in CSV export, and on
    /// each record behind a relationship `Select`'s options (GH #108). Note
    /// both hooks default-deny: a resource used as a relationship target
    /// must allow `can_view_any` **and** `can_view` (overriding one does not
    /// imply the other). The list page deliberately checks only
    /// `can_view_any` (GH #86): `can_view` is an in-memory Rust predicate
    /// that cannot run in SQL, and filtering rows after cursor pagination
    /// would mislabel pages (holes, wrong Next/Prev). Row-level visibility
    /// that must hold on the list belongs in [`Self::query`] (the tenancy
    /// seam, ADR-0002), which every loader — list, edit, delete, bulk,
    /// export — already funnels through.
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

    /// Sidebar entry for the resource.
    fn navigation() -> NavigationItem {
        NavigationItem::from_resource::<Self>()
    }

    /// Create a new record from form values.
    ///
    /// The `Panel` create handler validates `required`/`email` inline and checks
    /// `Resource::can_create` before calling this, inside a framework-owned
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
///
/// Delegates to `heck::ToKebabCase` (GH #139; the hand-rolled scanner matched
/// heck on every Rust-identifier shape — digits, acronym runs — so slugs are
/// unchanged). Underscores now split words too (`Audit_Log` → `audit-log`,
/// previously `audit_log`): name resources without underscores or override
/// [`Resource::slug`](crate::Resource::slug).
fn kebab_case(name: &str) -> String {
    use heck::ToKebabCase;
    name.to_kebab_case()
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
        // The heck delegate (GH #139): digit boundaries match the old scanner,
        // and underscores now split words — pinned so a heck upgrade cannot
        // silently change slugs.
        assert_eq!(kebab_case("User2FA"), "user2-fa");
        assert_eq!(kebab_case("Blog_Post"), "blog-post");
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
        // GH #156: columns need distinct names — title + status, not one
        // field twice.
        let tasks_table = Table::<Task>::r#for(&cx).id(|t| t.id.to_string()).columns((
            TextColumn::r#for(Task::fields().title(), |t: &Task| t.title.clone()).searchable(),
            TextColumn::r#for(Task::fields().status(), |t: &Task| t.status.clone()).sortable(),
        ));
        // Use dummy rows for render check (no DB) — keyed by row.id
        let rows = vec![
            Task {
                id: uuid::Uuid::new_v4(),
                title: "Ada".to_string(),
                status: "draft".to_string(),
                featured: false,
                created_at: jiff::Timestamp::now(),
            },
            Task {
                id: uuid::Uuid::new_v4(),
                title: "Bob".to_string(),
                status: "published".to_string(),
                featured: true,
                created_at: jiff::Timestamp::now(),
            },
        ];
        let page: TablePage<Task> = rows.clone().into();
        let html = tasks_table
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
        // Searchable columns render no extra header chrome (GH #154): the
        // search input is the affordance. Sortable ones carry the inactive
        // `arrow-up-down` with `aria-sort="none"`.
        assert!(
            !html.contains("Prefix search matches this column"),
            "searchable headers must not render a loupe, got {html}"
        );
        assert!(
            html.contains("aria-sort=\"none\""),
            "missing sortable indicator in {html}"
        );
        assert!(
            html.contains("cursor-pointer"),
            "missing sortable cursor-pointer in {html}"
        );
        assert!(html.contains("Title"), "missing Title header in {html}");
        assert!(html.contains("Status"), "missing Status header in {html}");
        for row in &rows {
            assert!(
                html.contains(&row.title),
                "missing row title {} in {html}",
                row.title
            );
        }
    }

    #[tokio::test]
    async fn interactive_false_renders_a_static_preview() {
        // GH #151: a demo/preview table renders declarations, not the
        // interactions — no search toolbar, no sort link, no sort state.
        let cx = CxTestBuilder::new().build();
        let preview = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(
                TextColumn::r#for(User::fields().name(), |u| u.name.clone())
                    .searchable()
                    .sortable(),
            )
            .interactive(false);
        let rows = vec![User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        }];
        let page: TablePage<User> = rows.into();
        let html = preview
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("name=\"q\"")
                && !html.contains("aria-sort")
                && !html.contains("sort=name"),
            "static preview must not render interactive chrome, got {html}"
        );
        assert!(
            html.contains("Name") && html.contains("Ada"),
            "static preview must still render labels and rows, got {html}"
        );
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
        // Bulk form keeps the hidden `ids` transport (GH #151 removed the
        // visible free-text fallback) and a submit that ships disabled until
        // `bulk.js` sees a checked row.
        assert!(
            html.contains("data-bulk-form"),
            "missing bulk form in {html}"
        );
        assert!(
            html.contains("name=\"ids\"")
                && html.contains("data-bulk-ids")
                && !html.contains("ids comma-separated"),
            "missing hidden ids transport in {html}"
        );
        assert!(
            html.contains("data-bulk-submit=\"\"") && html.contains("disabled=\"\""),
            "the bulk submit must ship disabled in {html}"
        );
        assert!(
            html.contains("Bulk Delete"),
            "missing bulk button in {html}"
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
        // The hidden transport carries the composed value for auto-apply; the
        // free-text input + Apply button survive only as the `<noscript>`
        // fallback (GH #151).
        assert!(
            html.contains("data-filters-transport")
                && html.contains("name=\"filters\"")
                && html.contains("status:published"),
            "missing hidden filters transport in {html}"
        );
        assert!(
            html.contains("<noscript>") && html.contains("Apply filters"),
            "missing no-JS filter fallback in {html}"
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
    async fn unknown_filter_warns_on_an_empty_page_too() {
        // GH #93 follow-up: the zero-rows branch returned before the warning
        // banner rendered, so a typo'd filter looked like an honest "no
        // results" on an empty table.
        let cx = CxTestBuilder::new().build();
        let html = status_table(&cx)
            .render_with_state(
                &cx,
                Vec::new().into(),
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
            "empty page must still warn about ignored filters, got {html}"
        );
    }

    #[tokio::test]
    async fn empty_clear_links_preserve_the_untouched_state() {
        // The empty-state link used to rebuild the URL from `sort` alone:
        // `group_by` was always dropped, and with a search + filters active
        // the "Clear search" link also cleared the filters.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t| t.title.clone()).sortable())
            .filters(SelectFilter::r#for(
                Task::fields().status(),
                vec!["published".to_string()],
            ))
            .group_by("status", |t| t.status.clone());
        let state = TableState {
            search: Some("Hello".to_string()),
            filters: HashMap::from([("status".to_string(), "published".to_string())]),
            sort: Some(Sort {
                column: "title".to_string(),
                descending: true,
            }),
            group_by: Some("status".to_string()),
            ..TableState::default()
        };
        let html = tbl
            .render_with_state(&cx, Vec::new().into(), &state, "/admin/tasks")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let clear = last_link_named(&html, "Clear search");
        assert!(
            clear.contains("sort=title"),
            "clear search must keep sort: {clear}"
        );
        assert!(
            clear.contains("dir=desc"),
            "clear search must keep dir: {clear}"
        );
        assert!(
            clear.contains("filters="),
            "clear search must keep filters: {clear}"
        );
        assert!(
            clear.contains("group_by=status"),
            "clear search must keep group_by: {clear}"
        );
        assert!(!clear.contains("q="), "clear search must drop q: {clear}");

        let state = TableState {
            filters: HashMap::from([("status".to_string(), "published".to_string())]),
            sort: Some(Sort {
                column: "title".to_string(),
                descending: true,
            }),
            group_by: Some("status".to_string()),
            ..TableState::default()
        };
        let html = tbl
            .render_with_state(&cx, Vec::new().into(), &state, "/admin/tasks")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // The filter bar renders a "Clear filters" link earlier in the page;
        // the empty-cell one is the subject here.
        let clear = last_link_named(&html, "Clear filters");
        assert!(
            clear.contains("sort=title"),
            "clear filters must keep sort: {clear}"
        );
        assert!(
            clear.contains("group_by=status"),
            "clear filters must keep group_by: {clear}"
        );
        assert!(
            !clear.contains("filters="),
            "clear filters must drop filters: {clear}"
        );
    }

    fn last_link_named<'a>(html: &'a str, label: &str) -> &'a str {
        html.rsplit('<')
            .find(|chunk| chunk.contains(label))
            .unwrap_or_else(|| panic!("missing {label} link in {html}"))
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

    #[tokio::test]
    async fn table_load_rejects_both_cursors() {
        // GH #155: `?after=` + `?before=` together must fail loudly instead of
        // silently preferring `after` (the GH #93 fail-open family). The
        // failure carries the `CursorDecodeError` marker so the retry link
        // drops pagination (GH #110).
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["Ada", "Bob"] {
            toasty::create!(User {
                name: name.to_string()
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let tbl = Table::<User>::new()
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u: &User| {
                u.name.clone()
            }))
            .paginate(1);
        // A valid cursor token: the first page of two rows has a next page.
        let first = tbl
            .load(
                &cx,
                toasty::stmt::Query::<List<User>>::all(),
                &TableState::default(),
            )
            .await
            .unwrap();
        let cursor = first
            .next_cursor
            .clone()
            .expect("page 1 must have a cursor");
        // Sanity: a single cursor still loads.
        let state = TableState {
            after: Some(cursor.clone()),
            ..TableState::default()
        };
        let second = tbl
            .load(&cx, toasty::stmt::Query::<List<User>>::all(), &state)
            .await
            .unwrap();
        assert_eq!(second.rows.len(), 1);
        // Both cursors together fail with the cursor marker — no silent
        // precedence for whichever comes first.
        let state = TableState {
            after: Some(cursor.clone()),
            before: Some(cursor),
            ..TableState::default()
        };
        let err = tbl
            .load(&cx, toasty::stmt::Query::<List<User>>::all(), &state)
            .await
            .expect_err("after+before must fail loudly");
        assert!(
            err.downcast_ref::<crate::cursor::CursorDecodeError>()
                .is_some(),
            "conflict must carry the cursor marker for the retry contract, got {err}"
        );
    }

    #[test]
    fn table_search_expr_ors_across_searchable_columns() {
        let cx = CxTestBuilder::new().build();
        // GH #156: distinct names — title + status, not one field twice.
        let tasks_table = Table::<Task>::r#for(&cx).columns((
            TextColumn::r#for(Task::fields().title(), |t| t.title.clone()).searchable(),
            TextColumn::r#for(Task::fields().status(), |t| t.status.clone()).searchable(),
        ));
        assert!(tasks_table.search_expr("Ada").is_some());
        assert!(tasks_table.search_expr("").is_none());
        assert!(tasks_table.search_expr("   ").is_none());
        let table_none = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(table_none.search_expr("Ada").is_none());
    }

    #[test]
    fn table_order_by_returns_first_sortable() {
        let cx = CxTestBuilder::new().build();
        // GH #156: distinct names — title sortable + status plain.
        let tasks_table = Table::<Task>::r#for(&cx).columns((
            TextColumn::r#for(Task::fields().title(), |t| t.title.clone()).sortable(),
            TextColumn::r#for(Task::fields().status(), |t| t.status.clone()),
        ));
        assert!(tasks_table.order_by(false).is_some());
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
    fn to_csv_defuses_cr_lf_led_formula_cells() {
        // GH #145: spreadsheets treat CR/LF- and tab-led payloads as formulas
        // even when the dangerous character does not start the raw cell, so
        // the defuse test skips leading whitespace/controls. CR/LF-led cells
        // are RFC4180-quoted (they carry a newline); a tab-led cell has no
        // quote/comma/newline and stays bare — either way the `'` leads the
        // defused content.
        let cx = CxTestBuilder::new().build();
        let csv_table = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        for (payload, defused) in [
            ("\r=1+1", "'\r=1+1"),
            ("\n@cmd", "'\n@cmd"),
            ("\t+1+1", "'\t+1+1"),
            (" \r=HYPERLINK(1,2)", "' \r=HYPERLINK(1,2)"),
            // BOM/ZWSP are neither whitespace nor control: cover the
            // format-character gap explicitly.
            ("\u{FEFF}=1+1", "'\u{FEFF}=1+1"),
            ("\u{200B}@cmd", "'\u{200B}@cmd"),
        ] {
            let rows = vec![User {
                id: uuid::Uuid::nil(),
                name: payload.to_string(),
            }];
            let csv = csv_table.to_csv(&rows.into());
            assert!(
                csv.contains(defused),
                "CR/LF-led formula payload {payload:?} must be defused to {defused:?}, got {csv:?}"
            );
        }
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
    #[should_panic(expected = "duplicate column name")]
    fn duplicate_column_name_panics_on_field_computed_collision() {
        // GH #156: computed("Status") derives name "status", colliding with
        // the field column's name — the Column::name namespace must stay
        // unique even though computeds are never sortable today (GH #101).
        let _ = Table::<Task>::new().columns((
            TextColumn::r#for(Task::fields().status(), |t: &Task| t.status.clone()).sortable(),
            TextColumn::computed("Status", |t: &Task| t.status.clone()),
        ));
    }

    #[test]
    #[should_panic(expected = "duplicate column name")]
    fn duplicate_column_name_panics_on_case_only_computed_collision() {
        // GH #156: computed names are label.to_lowercase(), so labels
        // differing only by case still collide.
        let _ = Table::<User>::new().columns((
            TextColumn::computed("Status", |u: &User| u.name.clone()),
            TextColumn::computed("STATUS", |u: &User| u.name.clone()),
        ));
    }

    #[test]
    #[should_panic(expected = "duplicate column name")]
    fn duplicate_column_name_panics_on_duplicate_field() {
        // GH #156: same guard covers two bindings of one field.
        let _ = Table::<User>::new().columns((
            TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone()),
            TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone()),
        ));
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
        let (back, malformed) = parse_filters_param(&param);
        assert!(
            malformed.is_empty(),
            "round-trip must not invent malformed segments, got {malformed:?}"
        );
        assert_eq!(back.get("q").map(String::as_str), Some("a,b"));
        assert_eq!(back.get("tag").map(String::as_str), Some("x:y%z"));
        // Duplicate keys keep the first, never silent last-wins.
        let (dup, dup_malformed) = parse_filters_param("k:a,k:b");
        assert_eq!(dup.get("k").map(String::as_str), Some("a"));
        assert!(dup_malformed.is_empty());
        // Legacy plain values still parse.
        let (legacy, legacy_malformed) = parse_filters_param("status:published, featured:true");
        assert_eq!(legacy.get("status").map(String::as_str), Some("published"));
        assert!(legacy_malformed.is_empty());
        // Blank segments stay silent (the boundary between "skipped" and
        // "malformed"); space-padded keys still parse.
        let (blank, blank_bad) = parse_filters_param(",,status:draft");
        assert!(
            blank_bad.is_empty(),
            "blank segments are skipped, got {blank_bad:?}"
        );
        assert_eq!(blank.get("status").map(String::as_str), Some("draft"));
        // Colon-less and empty-value segments are malformed, not dropped (GH #148).
        let (ok, bad) = parse_filters_param("foobar,:val,key:,status:published");
        assert_eq!(ok.get("status").map(String::as_str), Some("published"));
        assert_eq!(
            bad,
            ["foobar".to_string(), ":val".to_string(), "key:".to_string()]
        );
        // Round-trip keeps them flagged: filters_param re-emits them verbatim
        // (last, after the sorted pairs), so the next parse flags them again.
        let state = TableState {
            filters: ok,
            malformed_filters: bad.clone(),
            ..TableState::default()
        };
        let param = state.filters_param().expect("must serialize");
        let (again_ok, again_bad) = parse_filters_param(&param);
        assert_eq!(again_bad, bad, "malformed segments must round-trip");
        assert_eq!(
            again_ok.get("status").map(String::as_str),
            Some("published")
        );
        // Percent-escape round-trips per component (case-insensitive decode).
        for raw in ["a,b", "x:y%z", "100%", "a:b:c", "%3A%2C%25"] {
            let enc = encode_filter_component(raw);
            assert_eq!(decode_filter_component(&enc), raw, "round-trip {raw:?}");
        }
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
        assert_eq!(
            html.matches("aria-busy=\"true\"").count(),
            2,
            "busy must ride on the morph boundary and the table root (GH #160), got {html}"
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

    /// Fully populated projection source (GH #153): every intent projects
    /// from this through the real parser (`from_cx`), asserting the typed
    /// delta — state, not URL bytes.
    fn populated_state() -> TableState {
        TableState {
            search: Some("Ada".to_string()),
            sort: Some(Sort {
                column: "name".to_string(),
                descending: true,
            }),
            after: Some("after-cur".to_string()),
            before: Some("before-cur".to_string()),
            filters: HashMap::from([("status".to_string(), "published".to_string())]),
            malformed_filters: vec!["bogus".to_string()],
            group_by: Some("status".to_string()),
            delete: Some("row-1".to_string()),
            open: Some(false),
        }
    }

    /// Project through an intent and re-parse the URL with the real parser.
    fn reparse(url: &str) -> TableState {
        let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
        TableState::from_cx(&cx_with_query(query))
    }

    #[test]
    fn projection_list_url_round_trips_full_state() {
        // GH #153: full state including cursors; never `delete`/`open`.
        let source = populated_state();
        let mut expected = source.clone();
        expected.delete = None;
        expected.open = None;
        assert_eq!(reparse(&source.list_url("/admin/users")), expected);
    }

    #[test]
    fn projection_without_search_drops_query() {
        // GH #153: drops `q` (and its result set's cursors + dialog); keeps
        // the `filters` transport including malformed segments.
        let source = populated_state();
        let mut expected = source.clone();
        expected.search = None;
        expected.after = None;
        expected.before = None;
        expected.delete = None;
        expected.open = None;
        assert_eq!(reparse(&source.without_search("/admin/users")), expected);
    }

    #[test]
    fn projection_without_filters_drops_filters() {
        // GH #153: drops `filters` and malformed segments (and their result
        // set's cursors + dialog); keeps the search term.
        let source = populated_state();
        let mut expected = source.clone();
        expected.filters = HashMap::new();
        expected.malformed_filters = Vec::new();
        expected.after = None;
        expected.before = None;
        expected.delete = None;
        expected.open = None;
        assert_eq!(reparse(&source.without_filters("/admin/users")), expected);
    }

    #[test]
    fn projection_without_cursor_drops_pagination() {
        // GH #153 (GH #110): drops `after`/`before`; keeps everything else.
        let source = populated_state();
        let mut expected = source.clone();
        expected.after = None;
        expected.before = None;
        expected.delete = None;
        expected.open = None;
        assert_eq!(reparse(&source.without_cursor("/admin/users")), expected);
    }

    #[test]
    fn projection_with_after_sets_forward_cursor() {
        // GH #153: full state + `after`, drops `before` and the dialog.
        let source = populated_state();
        let mut expected = source.clone();
        expected.after = Some("tok2".to_string());
        expected.before = None;
        expected.delete = None;
        expected.open = None;
        assert_eq!(
            reparse(&source.with_after("/admin/users", "tok2")),
            expected
        );
    }

    #[test]
    fn projection_with_before_sets_backward_cursor() {
        // GH #153: full state + `before`, drops `after` and the dialog.
        let source = populated_state();
        let mut expected = source.clone();
        expected.after = None;
        expected.before = Some("tok2".to_string());
        expected.delete = None;
        expected.open = None;
        assert_eq!(
            reparse(&source.with_before("/admin/users", "tok2")),
            expected
        );
    }

    #[test]
    fn projection_sorted_by_replaces_sort() {
        // GH #153: replaces `sort`/`dir`, drops cursors and the dialog.
        let source = populated_state();
        let mut expected = source.clone();
        expected.sort = Some(Sort {
            column: "title".to_string(),
            descending: false,
        });
        expected.after = None;
        expected.before = None;
        expected.delete = None;
        expected.open = None;
        assert_eq!(
            reparse(&source.sorted_by("/admin/users", "title", false)),
            expected
        );
    }

    #[test]
    fn projection_with_delete_dialog_adds_key() {
        // GH #153 (GH #151): full state including cursors + `delete=key`;
        // never `open`.
        let source = populated_state();
        let mut expected = source.clone();
        expected.delete = Some("row-9".to_string());
        expected.open = None;
        assert_eq!(
            reparse(&source.with_delete_dialog("/admin/users", "row-9")),
            expected
        );
    }
}
