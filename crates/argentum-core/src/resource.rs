//! `Resource` — maps one Toasty [`Model`] to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, pages, and navigation. See
//! `CONTEXT.md` and ADR-0002.

use std::marker::PhantomData;
use std::sync::Arc;

use toasty::stmt::{Expr, List, OrderByExpr};
use topcoat::context::Cx;
use topcoat::router::{Href, HrefParams, HrefQueries, HrefTarget};
use topcoat::{Result, view::*};

use crate::schema::{FieldLens, Schema, lens_field_name_and_label, pk_tie_breakers};

/// Text column bound to a typed lens. `TextColumn::for(User::fields().name())`
/// fails if the column does not exist (ADR-0001).
#[derive(Debug, Clone)]
pub struct TextColumn<M> {
    path: FieldLens<M, String>,
    name: String,
    label: String,
    searchable: bool,
    sortable: bool,
}

impl<M> TextColumn<M>
where
    M: toasty::schema::Model,
{
    pub fn for_lens(path: FieldLens<M, String>) -> Self {
        let (field_name, label) = lens_field_name_and_label(path.clone());
        Self {
            path,
            name: field_name,
            label,
            searchable: false,
            sortable: false,
        }
    }

    pub fn r#for(path: FieldLens<M, String>) -> Self {
        Self::for_lens(path)
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

    pub fn header_class(&self) -> String {
        // Legacy helper kept for compat; new Table chrome uses Token classes directly.
        let mut cls = "text-muted-foreground".to_string();
        if self.searchable {
            cls.push_str(" searchable");
        }
        if self.sortable {
            cls.push_str(" sortable cursor-pointer");
        }
        cls
    }

    pub fn to_search_expr(&self, term: &str) -> Option<Expr<bool>> {
        let t = term.trim();
        if !self.searchable || t.is_empty() {
            return None;
        }
        Some(self.path.clone().starts_with(t.to_string()))
    }

    pub fn to_order_by(&self) -> Option<OrderByExpr> {
        if self.sortable {
            // PK tie-breaker lives in Table::order_bys() (deterministic pagination).
            Some(self.path.clone().asc())
        } else {
            None
        }
    }
}

/// Column enum — Phase 1: only `Text`. Will generalize to `Number`, `Badge`, etc. later.
#[derive(Debug, Clone)]
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

    pub fn header_class(&self) -> String {
        match self {
            Column::Text(c) => c.header_class(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Column::Text(c) => &c.name,
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

/// Row identity — `key: &row.id` per CONTEXT.md. Simple string id for Phase 1.
///
/// Stringly-typed `GetField` contradicts ADR-0001 typed lens and is
/// intentional tech debt (see GH #10). Will become a typed projection
/// (`Column` holding `Fn(&M)->String` or lens-aware `Cell`); panic-on-unknown
/// now preserves typo visibility without over-design.
pub trait HasId {
    fn id_string(&self) -> String;
}

/// Field accessor for Table cell rendering. Minimal stringly-typed for Phase 1,
/// will be replaced by typed lens projection (see `HasId` doc, GH #10).
pub trait GetField {
    fn get_field(&self, name: &str) -> String;
}

/// Table description of a `Resource`'s list view. Declares columns and how they map to queries.
#[derive(Debug)]
pub struct Table<M> {
    columns: Vec<Column<M>>,
    pagination: bool,
    show_skeleton: bool,
    _marker: PhantomData<M>,
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
            pagination: false,
            show_skeleton: false,
            _marker: PhantomData,
        }
    }

    /// Create a table for the given model. `cx` is reserved for future tenancy/policy scoping.
    pub fn r#for(_cx: &Cx) -> Self {
        Self::new()
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
        let mut exprs = self.columns.iter().filter_map(|c| match c {
            Column::Text(col) => col.to_search_expr(t),
        });
        let first = exprs.next()?;
        Some(exprs.fold(first, |acc, e| acc.or(e)))
    }

    /// First sortable column's order_by (asc). Deterministic pagination requires
    /// a PK tie-breaker — use [`Self::order_bys`] for the full ordering.
    pub fn order_by(&self) -> Option<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        self.columns.iter().find_map(|c| match c {
            Column::Text(col) => col.to_order_by(),
        })
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
        let Some(first) = self.order_by() else {
            return Vec::new();
        };
        let mut out = vec![first];
        out.extend(pk_tie_breakers::<M>());
        out
    }

    /// Render the table for the given rows. Header shows searchable/sortable indicators;
    /// rows are keyed by `row.id` per CONTEXT.md.
    ///
    /// Beautiful chrome: `rounded-xl border border-border overflow-hidden` container,
    /// `table` primitives (w-full, border-border, text-muted-foreground, hover:bg-foreground/5),
    /// sortable `cursor-pointer`, searchable/sortable indicators with `aria-sort`,
    /// pagination chrome when `pagination` is set, EmptyState via card when `rows` is empty,
    /// skeleton rows when `skeleton` is set.
    /// Delegates visually to `argentum-ui` token classes (`bg-background`, `border-border`,
    /// `shadow-sm`, `text-muted-foreground`) — no raw colors, no `ac-*`.
    pub async fn render(&self, cx: &Cx, rows: &[M]) -> Result<View>
    where
        M: toasty::schema::Model + HasId + GetField + std::fmt::Debug + Send + Sync + 'static,
    {
        if self.show_skeleton {
            return view! {
                cx =>
                <div class="rounded-xl border border-border overflow-hidden">
                    <div class="w-full overflow-x-auto">
                        <table class="w-full caption-bottom border-collapse text-sm">
                            <thead class="[&_tr]:border-b">
                                <tr class="border-b border-border transition-colors hover:bg-foreground/5">
                                    for col in &self.columns {
                                        <th class="h-10 px-3 text-left align-middle font-medium whitespace-nowrap text-muted-foreground">(col.label())</th>
                                    }
                                </tr>
                            </thead>
                            <tbody class="[&_tr:last-child]:border-0">
                                for _ in 0..3 {
                                    <tr class="border-b border-border transition-colors hover:bg-foreground/5">
                                        for _ in &self.columns {
                                            <td class="p-3 align-middle whitespace-nowrap"><div class="animate-pulse rounded-md bg-foreground/10 h-4 w-full"></div></td>
                                        }
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>
                </div>
            };
        }

        if rows.is_empty() {
            return view! {
                cx =>
                <div class="rounded-xl border border-border overflow-hidden">
                    <div class="w-full overflow-x-auto">
                        <table class="w-full caption-bottom border-collapse text-sm">
                            <thead class="[&_tr]:border-b">
                                <tr class="border-b border-border transition-colors hover:bg-foreground/5">
                                    for col in &self.columns {
                                        <th class=(if col.is_sortable() { "h-10 px-3 text-left align-middle font-medium whitespace-nowrap text-muted-foreground cursor-pointer hover:bg-foreground/5" } else { "h-10 px-3 text-left align-middle font-medium whitespace-nowrap text-muted-foreground" }) aria-sort=(if col.is_sortable() { Some("none") } else { None })>
                                            (col.label())
                                            if col.is_searchable() {
                                                <span class="ml-2 text-muted-foreground">"⌕"</span>
                                            }
                                            if col.is_sortable() {
                                                <span class="ml-2">"↕"</span>
                                            }
                                        </th>
                                    }
                                </tr>
                            </thead>
                        </table>
                    </div>
                    <div class="px-6 py-16 text-center">
                        <div class="flex flex-col items-center gap-4">
                            <p class="text-sm text-muted-foreground">"No records found"</p>
                            <p class="text-sm text-muted-foreground">"No search results"</p>
                            <button class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent bg-primary text-primary-foreground shadow-xs hover:bg-primary/90 active:bg-primary/80 h-9 gap-2 rounded-lg px-4 text-sm">"Create record"</button>
                        </div>
                    </div>
                    if self.pagination {
                        <div class="border-t border-border p-3">
                            <nav aria-label="pagination" class="@container mx-auto flex w-full justify-center">
                                <ul class="flex flex-row flex-wrap items-center justify-center gap-1">
                                    <li><a aria-current="page" class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-border text-foreground shadow-xs hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base">"1"</a></li>
                                    <li><a class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base">"2"</a></li>
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
                <div class="w-full overflow-x-auto">
                    <table class="w-full caption-bottom border-collapse text-sm">
                        <thead class="[&_tr]:border-b">
                            <tr class="border-b border-border transition-colors hover:bg-foreground/5">
                                for col in &self.columns {
                                    <th class=(if col.is_sortable() { "h-10 px-3 text-left align-middle font-medium whitespace-nowrap text-muted-foreground cursor-pointer hover:bg-foreground/5" } else { "h-10 px-3 text-left align-middle font-medium whitespace-nowrap text-muted-foreground" }) aria-sort=(if col.is_sortable() { Some("none") } else { None })>
                                        (col.label())
                                        if col.is_searchable() {
                                            <span class="ml-2 text-muted-foreground">"⌕"</span>
                                        }
                                        if col.is_sortable() {
                                            <span class="ml-2">"↕"</span>
                                        }
                                    </th>
                                }
                            </tr>
                        </thead>
                        <tbody class="[&_tr:last-child]:border-0">
                            for row in rows {
                                <tr key=(row.id_string()) class="border-b border-border transition-colors hover:bg-foreground/5">
                                    for col in &self.columns {
                                        <td class="p-3 align-middle whitespace-nowrap">(row.get_field(col.name()))</td>
                                    }
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
                if self.pagination {
                    <div class="border-t border-border p-3">
                        <nav aria-label="pagination" class="@container mx-auto flex w-full justify-center">
                            <ul class="flex flex-row flex-wrap items-center justify-center gap-1">
                                <li><a aria-current="page" class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-border text-foreground shadow-xs hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base">"1"</a></li>
                                <li><a class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base">"2"</a></li>
                                <li><a class="inline-flex shrink-0 items-center justify-center border font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10 size-9 rounded-lg text-base">"3"</a></li>
                            </ul>
                        </nav>
                    </div>
                }
            </div>
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
                <div class="grid w-full grid-cols-[0_1fr] items-start gap-y-1 rounded-lg border bg-background px-4 py-3 text-sm border-destructive/50 text-destructive has-[>svg]:grid-cols-[1rem_1fr] has-[>svg]:gap-x-3 [&>svg]:size-4 [&>svg]:translate-y-0.5">
                    <p class="col-start-2 font-medium tracking-tight">"Failed to load"</p>
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

/// Sidebar entry derived from a `Resource` (see `CONTEXT.md`).
pub struct NavigationItem {
    pub label: String,
    pub url: String,
    pub href_check: Option<Arc<dyn Fn(&Cx) -> bool + Send + Sync>>,
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

impl Default for NavigationItem {
    fn default() -> Self {
        Self {
            label: String::new(),
            url: String::new(),
            href_check: None,
        }
    }
}

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
        let check = Arc::new(move |cx: &Cx| href.is_current(cx))
            as Arc<dyn Fn(&Cx) -> bool + Send + Sync>;
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

    impl HasId for User {
        fn id_string(&self) -> String {
            self.id.to_string()
        }
    }

    impl GetField for User {
        fn get_field(&self, name: &str) -> String {
            match name {
                "name" => self.name.clone(),
                "id" => self.id.to_string(),
                _ => panic!(
                    "GetField: unknown column '{}' for {}",
                    name,
                    std::any::type_name::<Self>()
                ),
            }
        }
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
        let users = NavigationItem {label: "Users".to_string(),
            url: "/admin".to_string(),
            href_check: None,
        };
        let showcase = NavigationItem {label: "Showcase".to_string(),
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
        let item = NavigationItem {label: "Showcase".to_string(),
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
        let col = TextColumn::r#for(User::fields().name()).searchable();
        assert!(
            col.to_search_expr("Ada").is_some(),
            "searchable should produce expr"
        );
        assert!(
            TextColumn::r#for(User::fields().name())
                .to_search_expr("Ada")
                .is_none(),
            "non-searchable should be None"
        );
    }

    #[test]
    fn text_column_sortable_produces_order_by() {
        let col = TextColumn::r#for(User::fields().name()).sortable();
        assert!(
            col.to_order_by().is_some(),
            "sortable should produce order_by"
        );
        assert!(
            TextColumn::r#for(User::fields().name())
                .to_order_by()
                .is_none(),
            "non-sortable should be None"
        );
    }

    #[tokio::test]
    async fn table_for_columns_renders_with_keyed_rows() {
        let cx = CxTestBuilder::new().build();
        let table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name()).searchable(),
            TextColumn::r#for(User::fields().name()).sortable(),
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
        let html = table.render(&cx, &rows).await.unwrap().render(&cx);
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
        let col = TextColumn::r#for(User::fields().name()).searchable();
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
        let table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name()).searchable(),
            TextColumn::r#for(User::fields().name()).searchable(),
        ));
        assert!(table.search_expr("Ada").is_some());
        assert!(table.search_expr("").is_none());
        assert!(table.search_expr("   ").is_none());
        let table_none =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()));
        assert!(table_none.search_expr("Ada").is_none());
    }

    #[test]
    fn table_order_by_returns_first_sortable() {
        let cx = CxTestBuilder::new().build();
        let table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name()).sortable(),
            TextColumn::r#for(User::fields().name()),
        ));
        assert!(table.order_by().is_some());
        let table_none =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()));
        assert!(table_none.order_by().is_none());
    }

    #[test]
    fn table_order_bys_includes_pk_tie_breaker() {
        let cx = CxTestBuilder::new().build();
        let table =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()).sortable());
        let orders = table.order_bys();
        // first is sortable, second is PK asc for deterministic pagination
        assert_eq!(
            orders.len(),
            2,
            "sortable + PK tie-breaker expected, got {orders:?}"
        );
        // No sortable → empty
        let table_none =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()));
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
        let table =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()).sortable());
        let mut db = crate::db::db(&cx);
        let mut query = User::all();
        for ord in table.order_bys() {
            query = query.order_by(ord);
        }
        let rows = query.exec(&mut db).await.unwrap();
        assert_eq!(rows.len(), 2);
        // With PK tie-breaker asc, order is deterministic by id
        assert!(
            rows[0].id.to_string() < rows[1].id.to_string()
                || rows[0].id == a.id && rows[1].id == b.id
                || rows[0].id == b.id && rows[1].id == a.id,
            "rows should be ordered deterministically"
        );
        // Ensure stable ordering is by PK asc (lowest id first)
        let expected_first = if a.id.to_string() < b.id.to_string() {
            a.id
        } else {
            b.id
        };
        assert_eq!(rows[0].id, expected_first);
    }
}
