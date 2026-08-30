//! `Resource` — maps one Toasty [`Model`] to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, pages, and navigation. See
//! `CONTEXT.md` and ADR-0002.

use std::marker::PhantomData;
use std::sync::Arc;

use argentum_ui::{table, table_body, table_cell, table_head, table_header, table_row};
use toasty::stmt::{Expr, List, OrderByExpr};
use topcoat::context::Cx;
use topcoat::router::{Href, HrefParams, HrefQueries, HrefTarget};
use topcoat::{Result, view::*};

use crate::schema::{FieldLens, Schema, lens_field_name_and_label, pk_tie_breakers};

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
    path: FieldLens<M, String>,
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
            path,
            name: field_name,
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
        Some(self.path.clone().starts_with(t.to_string()))
    }

    pub fn to_order_by(&self, descending: bool) -> Option<OrderByExpr> {
        if self.sortable {
            // PK tie-breaker lives in Table::order_bys() (deterministic pagination).
            let path = self.path.clone();
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
/// 4-tuple limit is intentional: without variadic generics this is idiomatic
/// Rust — matches `IntoSchema` in `schema.rs`. Extending to 5+ columns adds
/// boilerplate for little gain; a macro is deferred until a real Resource
/// needs 5 columns.
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
pub struct Table<M> {
    columns: Vec<Column<M>>,
    row_key: Option<RowKey<M>>,
    pagination: bool,
    show_skeleton: bool,
    _marker: PhantomData<M>,
}

impl<M> std::fmt::Debug for Table<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("columns", &self.columns)
            .field("row_key", &self.row_key.is_some())
            .field("pagination", &self.pagination)
            .field("show_skeleton", &self.show_skeleton)
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
            row_key: None,
            pagination: false,
            show_skeleton: false,
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

    /// Enable pagination chrome (visual stub; query pagination is out of scope this slice).
    pub fn pagination(mut self, enabled: bool) -> Self {
        self.pagination = enabled;
        self
    }

    /// Enable skeleton placeholder rows (reserved for future `Boundary` `defer`).
    pub fn skeleton(mut self, enabled: bool) -> Self {
        self.show_skeleton = enabled;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
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

    /// First sortable column's order_by. Deterministic pagination requires
    /// a PK tie-breaker — use [`Self::order_bys`] for the full ordering.
    pub fn order_by(&self, descending: bool) -> Option<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        self.columns.iter().find_map(|c| c.to_order_by(descending))
    }

    /// Ordered list for the query: first sortable column asc + PK tie-breaker(s)
    /// for deterministic pagination (spec US10). Returns empty if no sortable
    /// column is declared; otherwise the PK field(s) are appended via
    /// `crate::schema::pk_tie_breakers` so every `M: Model` is stable
    /// regardless of whether the sortable column is unique.
    ///
    /// The tie-breaker is appended even if the sortable column is the PK
    /// itself — duplicate `order_by` on the same column is harmless and keeps
    /// the method branch-free.
    pub fn order_bys(&self) -> Vec<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        let Some(first) = self.order_by(false) else {
            return Vec::new();
        };
        let mut out = vec![first];
        out.extend(pk_tie_breakers::<M>());
        out
    }

    /// Render the table for the given rows. Header shows searchable/sortable indicators;
    /// rows are keyed by the projection declared via [`Self::id`] per `CONTEXT.md`,
    /// cells render via each column's typed projection.
    ///
    /// Beautiful chrome: `rounded-xl border border-border overflow-hidden` container,
    /// `table` primitives (w-full, border-border, text-muted-foreground, hover:bg-foreground/5),
    /// sortable `cursor-pointer`, searchable/sortable indicators with `aria-sort`,
    /// pagination chrome when `pagination` is set, EmptyState via card when `rows` is empty,
    /// skeleton rows when `skeleton` is set.
    /// Delegates visually to `argentum-ui` token classes (`bg-background`, `border-border`,
    /// `shadow-sm`, `text-muted-foreground`) — no raw colors, no `ac-*`.
    ///
    /// # Errors
    ///
    /// Errors when the table has no row key ([`Self::id`]) or no columns —
    /// row identity is not optional, and neither is something to show.
    pub async fn render(&self, cx: &Cx, rows: &[M]) -> Result<View>
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
        let head = self.render_thead(cx).await?;
        if self.show_skeleton {
            return view! {
                cx =>
                <div class="rounded-xl border border-border overflow-hidden">
                    table(
                        (head)
                        table_body(
                            for i in 0..3 {
                                table_row(
                                    key: i,
                                    for _ in &self.columns {
                                        table_cell(
                                            <div
                                                class="animate-pulse rounded-md bg-foreground/10 h-4 w-full"
                                            ></div>
                                        )
                                    }
                                )
                            }
                        )
                    )
                </div>
            };
        }

        if rows.is_empty() {
            return view! {
                cx =>
                <div class="rounded-xl border border-border overflow-hidden">
                    table((head))
                    <div class="px-6 py-16 text-center">
                        <div class="flex flex-col items-center gap-4">
                            <p class="text-sm text-muted-foreground">
                                "No records found"
                            </p>
                            <p class="text-sm text-muted-foreground">
                                "No search results"
                            </p>
                            <button
                                class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent bg-primary text-primary-foreground shadow-xs hover:bg-primary/90 active:bg-primary/80 h-9 gap-2 rounded-lg px-4 text-sm"
                            >
                                "Create record"
                            </button>
                        </div>
                    </div>
                    if self.pagination {
                        <div class="border-t border-border p-3">
                            <nav
                                aria-label="pagination"
                                class="@container mx-auto flex w-full justify-center"
                            >
                                <ul
                                    class="flex flex-row flex-wrap items-center justify-center gap-1"
                                >
                                    <li>
                                        <a
                                            aria-current="page"
                                            class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-border text-foreground shadow-xs hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base"
                                        >
                                            "1"
                                        </a>
                                    </li>
                                    <li>
                                        <a
                                            class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base"
                                        >
                                            "2"
                                        </a>
                                    </li>
                                </ul>
                            </nav>
                        </div>
                    }
                </div>
            };
        }

        view! {
            cx =>
            <div class="rounded-xl border border-border overflow-hidden">
                table(
                    (head)
                    table_body(
                        for row in rows {
                            let key = row_key(row);
                            let cells: Vec<String> =
                                self.columns.iter().map(|col| col.render_cell(row)).collect();
                            table_row(
                                key: key,
                                for cell in cells {
                                    table_cell((cell))
                                }
                            )
                        }
                    )
                )
                if self.pagination {
                    <div class="border-t border-border p-3">
                        <nav
                            aria-label="pagination"
                            class="@container mx-auto flex w-full justify-center"
                        >
                            <ul
                                class="flex flex-row flex-wrap items-center justify-center gap-1"
                            >
                                <li>
                                    <a
                                        aria-current="page"
                                        class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-border text-foreground shadow-xs hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base"
                                    >
                                        "1"
                                    </a>
                                </li>
                                <li>
                                    <a
                                        class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base"
                                    >
                                        "2"
                                    </a>
                                </li>
                                <li>
                                    <a
                                        class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base"
                                    >
                                        "3"
                                    </a>
                                </li>
                            </ul>
                        </nav>
                    </div>
                }
            </div>
        }
    }

    /// The shared column-header row — the single source of the `<thead>`
    /// markup (labels, searchable `⌕` / sortable `↕` indicators, `aria-sort`).
    /// Every render branch (skeleton / empty / rows) composes it, so an
    /// a11y or styling change happens once.
    async fn render_thead(&self, cx: &Cx) -> Result<View>
    where
        M: toasty::schema::Model,
    {
        let mut heads = Vec::with_capacity(self.columns.len());
        for col in &self.columns {
            let label = col.label().to_string();
            let sortable = col.is_sortable();
            let searchable = col.is_searchable();
            let head_class = if sortable {
                "cursor-pointer hover:bg-foreground/5"
            } else {
                ""
            };
            let aria_sort = if sortable { Some("none") } else { None };
            heads.push(view! { cx =>
                table_head(
                    attrs: attributes! {
                        class=(head_class)
                        aria-sort=(aria_sort)
                    },
                    (label.clone())
                    if searchable {
                        <span
                            role="img"
                            aria-label="Searchable column"
                            class="ml-2 inline-flex size-4 items-center justify-center align-middle text-base leading-none text-muted-foreground"
                        >
                            "⌕"
                        </span>
                    }
                    if sortable {
                        <button
                            type="button"
                            aria-label=(format!("Sort by {}", &label))
                            class="ml-2 inline-flex size-4 items-center justify-center align-middle text-base leading-none text-muted-foreground hover:text-foreground"
                        >
                            "↕"
                        </button>
                    }
                )
            }?);
        }
        view! { cx =>
            table_header(
                table_row(
                    for h in heads {
                        (h)
                    }
                )
            )
        }
    }

    /// Render an ErrorState (alert Destructive inside card) for loader failures.
    /// This is used via `Result` match in layout slot, not inside shard, so it
    /// survives Boundary swaps.
    pub async fn render_error(&self, cx: &Cx, message: &str) -> Result<View>
    where
        M: toasty::schema::Model,
    {
        let msg = message.to_string();
        view! {
            cx =>
            <div class="rounded-xl border border-border bg-background shadow-sm p-6">
                <div
                    class="grid w-full grid-cols-[0_1fr] items-start gap-y-1 rounded-lg border bg-background px-4 py-3 text-sm border-destructive/50 text-destructive has-[>svg]:grid-cols-[1rem_1fr] has-[>svg]:gap-x-3 [&>svg]:size-4 [&>svg]:translate-y-0.5"
                >
                    <p class="col-start-2 font-medium tracking-tight">
                        "Failed to load"
                    </p>
                    <div class="col-start-2 text-sm text-muted-foreground">(msg)</div>
                </div>
            </div>
        }
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
    /// Derive a sidebar entry from a `Resource` type, using the given panel mount prefix.
    ///
    /// Label is the `Model`'s type name without module path and with a
    /// trailing `s` for pluralisation (matching Filament's `User` → `Users`).
    /// URL is the panel prefix for the single-resource Phase 1 shell; multi-resource
    /// routing will become `{prefix}/<kebab-plural>` (ADR-0002 query seam
    /// handles scoping, Panel prefix owns the mount point).
    pub fn from_resource_with_prefix<R: Resource>(prefix: &str) -> Self {
        let model_name = std::any::type_name::<R::Model>();
        let short = model_name.rsplit("::").next().unwrap_or(model_name);
        let label = format!("{short}s");
        let url = if prefix.is_empty() {
            "/admin".to_string()
        } else {
            let trimmed = prefix.trim_matches('/').trim();
            if trimmed.is_empty() {
                "/admin".to_string()
            } else {
                format!("/{trimmed}")
            }
        };
        Self {
            label,
            url,
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
        let check =
            Arc::new(move |cx: &Cx| href.is_current(cx)) as Arc<dyn Fn(&Cx) -> bool + Send + Sync>;
        Self {
            label: label.into(),
            url: url.into(),
            href_check: Some(check),
        }
    }

    /// Derive a sidebar entry from a `Resource` type.
    ///
    /// Shorthand for `from_resource_with_prefix::<R>("/admin")` — kept for
    /// single-panel Phase 1 call sites. New code should use
    /// `from_resource_with_prefix` or `Panel::navigation_item`.
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

    /// Whether this item is current for the given request path (without query).
    ///
    /// Split from `is_current` so `Panel::render_shell` can stay testable
    /// without constructing a full `http::request::Parts` in `Cx`.
    pub fn is_current_path(&self, current_path: &str) -> bool {
        if current_path == self.url {
            return true;
        }
        if self.url == "/admin" {
            return false;
        }
        if current_path.starts_with(&self.url) {
            let rest = &current_path[self.url.len()..];
            return rest.starts_with('/');
        }
        false
    }
}

/// Maps one Toasty `Model` to its admin UI.
pub trait Resource: Sized + Send + Sync + 'static {
    /// The persisted model this resource administers.
    type Model: toasty::schema::Model;

    /// Base query — the **single seam** for tenancy/soft-delete scoping
    /// (ADR-0002). Every loader starts from this query.
    fn query(_cx: &Cx) -> <Self::Model as toasty::schema::Model>::Query<List<Self::Model>>
    where
        Self::Model: toasty::schema::Model,
    {
        <Self::Model as toasty::schema::Model>::wrap_query(
            toasty::stmt::Query::<List<Self::Model>>::all(),
        )
    }

    /// Description of the list view. Phase 1: stub.
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

        fn query(_cx: &Cx) -> <User as toasty::schema::Model>::Query<List<User>> {
            // Custom scoping example: only users named Ada
            User::filter(User::fields().name().eq("Ada"))
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
        assert_eq!(item.label, "Users");
        assert_eq!(item.url, "/admin");
    }

    #[test]
    fn navigation_item_is_current_path() {
        let users = NavigationItem {
            label: "Users".to_string(),
            url: "/admin".to_string(),
            href_check: None,
        };
        let showcase = NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
            href_check: None,
        };
        // exact
        assert!(users.is_current_path("/admin"));
        assert!(showcase.is_current_path("/admin/showcase"));
        // root exact-only — /admin/showcase should not highlight Users
        assert!(!users.is_current_path("/admin/showcase"));
        // slash-boundary — /admin/showcases should not highlight /admin/showcase
        assert!(!showcase.is_current_path("/admin/showcases"));
        assert!(!showcase.is_current_path("/admin/showcase-table"));
        // prefix with slash — sub-pages active
        assert!(showcase.is_current_path("/admin/showcase/table"));
        assert!(showcase.is_current_path("/admin/showcase/db"));
        // unrelated
        assert!(!users.is_current_path("/other"));
        assert!(!showcase.is_current_path("/admin"));
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
        assert!(
            no_columns.render(&cx, &rows).await.is_err(),
            "render without columns must error"
        );
        // Columns but no row key → error (replaces the old panic-on-unknown dispatch)
        let no_key = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(
            no_key.render(&cx, &rows).await.is_err(),
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
        let html = users_table.render(&cx, &rows).await.unwrap().render(&cx);
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
    fn table_order_bys_includes_pk_tie_breaker() {
        let cx = CxTestBuilder::new().build();
        let users_table = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());
        let orders = users_table.order_bys();
        // first is sortable, second is PK asc for deterministic pagination
        assert_eq!(
            orders.len(),
            2,
            "sortable + PK tie-breaker expected, got {orders:?}"
        );
        // No sortable → empty
        let table_none = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(
            table_none.order_bys().is_empty(),
            "non-sortable should have no order_bys"
        );
    }

    #[tokio::test]
    async fn table_order_bys_is_deterministic_for_pagination() {
        // Two rows with same name, different ids — order_bys with PK tie-breaker must be stable
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let a = toasty::create!(User { name: "Same" })
            .exec(&mut db)
            .await
            .unwrap();
        let b = toasty::create!(User { name: "Same" })
            .exec(&mut db)
            .await
            .unwrap();
        assert_ne!(a.id, b.id);
        let cx = CxTestBuilder::new().app_context(db).build();
        let users_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());
        let mut db = crate::db::db(&cx);
        let mut query = User::all();
        for ord in users_table.order_bys() {
            query = query.order_by(ord);
        }
        let rows = query.exec(&mut db).await.unwrap();
        assert_eq!(rows.len(), 2);
        // Ensure stable ordering is by PK asc (lowest id first)
        let expected_first = if a.id.to_string() < b.id.to_string() {
            a.id
        } else {
            b.id
        };
        assert_eq!(rows[0].id, expected_first);
    }
}
