//! The [`Table`] builder plus query planning (`filter_expr`/`search_expr`/`order_bys_for`).
//!
//! Rendering lives in [`render`](self::render), CSV export in [`export`](self::export).
//! One routine applies the declaration for both loaders (GH #210) and the
//! essentials check covers the action chrome the panel wires (GH #207).

use std::{marker::PhantomData, sync::Arc};

use toasty::stmt::{Expr, List, OrderByExpr};
use topcoat::{Result, context::Cx};

use super::{
    column::{IntoColumns, TextColumn},
    filter::{Filter, IntoFilters},
    state::{TablePage, TableState},
};

mod export;
mod render;

/// Row-key projection: reads the row identity off one model instance
/// (typically `|u| u.id.to_string()`). Toasty models are plain structs with
/// no instance→field reflection, so the key cannot be extracted generically
/// (upstream gap #119).
pub type RowKey<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

/// Group-label projection: reads a row's group off one model instance
/// (typically `|u| u.status.clone()`).
pub type GroupKey<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

/// Per-record action policy: reads which row actions one model instance allows
/// (GH #235). See [`Table::row_actions`].
pub type RowPolicy<M> = Arc<dyn Fn(&M) -> RowActions + Send + Sync>;

/// Which row actions one record may use (GH #235).
///
/// The per-record half of `TableChrome`: the chrome flags say which
/// affordances a resource declares, this says which of them the caller may use
/// on one loaded row. [`Table::row_actions`] stores the projection and the
/// renderer consults it per row — a denied action emits no link, and a row
/// denied `delete` renders its bulk checkbox `disabled` with the reason as its
/// accessible label, so the row can never enter the selection transport.
///
/// The panel derives one from the resource's
/// [`can_view`](crate::resource::Resource::can_view) /
/// [`can_update`](crate::resource::Resource::can_update) /
/// [`can_delete`](crate::resource::Resource::can_delete), pairing each action
/// with the same predicates its route checks, so a rendered affordance and the
/// route that answers it cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowActions {
    /// Whether the row renders its `View` link — the detail route's
    /// `can_view`.
    pub view: bool,
    /// Whether the row renders its `Edit` link — the edit route's `can_view`
    /// **and** `can_update` (GH #86).
    pub edit: bool,
    /// Whether the row renders its `Delete` link and an enabled bulk checkbox —
    /// the delete route's `can_view` **and** `can_delete` (GH #168).
    pub delete: bool,
}

impl RowActions {
    /// Every action allowed: what a table that declares no projection renders
    /// ([`Table::row_actions`]), so a hand-wired table shows exactly the chrome
    /// its `with_*` calls attached.
    pub const ALL: Self = Self {
        view: true,
        edit: true,
        delete: true,
    };
}

/// A named grouping a `Table` can render: `name` is the `?group_by=` value
/// the table accepts, `key` projects a row to its group label (GH #92).
pub struct GroupDef<M> {
    name: String,
    key: GroupKey<M>,
}

/// The action chrome a caller attaches to a [`Table`] after
/// [`Resource::table`](crate::resource::Resource::table) returned (GH #207).
///
/// `R::table(cx)` declares columns, filters, grouping and the row keys, but the
/// row actions are wired later — [`Table`] carries no delete/edit/view prefix
/// until `panel::wire_table_actions` attaches one. The record-key requirement
/// depends on that wiring, so the build-time declaration check has to be told
/// what the wiring will attach; `wire_table_actions` and the check derive it
/// from the same place, so the two cannot disagree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TableChrome {
    /// Whether the row renders a Delete action (which also enables bulk).
    pub(crate) delete: bool,
    /// Whether the row renders an Edit action.
    pub(crate) edit: bool,
    /// Whether the row renders a View action.
    pub(crate) view: bool,
}

impl TableChrome {
    /// Whether any row-level action link renders, and so needs a record key.
    ///
    /// The bulk-selection column needs it too, but rides the delete prefix (see
    /// `Table::bulk_enabled`), so it is covered here rather than named again.
    pub(crate) fn actions(&self) -> bool {
        self.delete || self.edit || self.view
    }
}

/// A [`TableState`] whose `group_by` has already been checked against the
/// table's declared grouping (GH #92, GH #153).
///
/// [`Table::normalize_state`] is the only constructor, so a seam that takes
/// one reads the pre-normalized `group_by` directly and cannot normalize a
/// second time — or forget to normalize at all (GH #224). The request entry
/// builds it once (the panel's list page, the `table_search` shard) and every
/// seam below takes this proof; the public render seams keep accepting a raw
/// `&TableState` and normalize it themselves, so an external page calling them
/// directly is unaffected.
///
/// Derefs to [`TableState`], so the seams that only read state keep their
/// `&TableState` signatures.
pub(crate) struct NormalizedState(TableState);

impl std::ops::Deref for NormalizedState {
    type Target = TableState;

    fn deref(&self) -> &TableState {
        &self.0
    }
}

/// How [`Table::order_bys_for`] falls back when `?sort=` names no sortable
/// column (GH #210): the one axis the list loader and the CSV export
/// legitimately disagree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderMode {
    /// List loading (GH #96): the PK fallback applies only to a paginated
    /// table, where toasty's cursor pagination needs a deterministic order.
    List,
    /// CSV export (GH #172): the chunked cursor walk needs a deterministic
    /// order whether or not the table paginates, so the PK fallback applies
    /// whenever no sortable column is declared.
    Export,
}

impl OrderMode {
    /// Whether the PK fallback applies, given whether the table paginates.
    fn falls_back_to_pk(self, paginated: bool) -> bool {
        match self {
            Self::List => paginated,
            Self::Export => true,
        }
    }
}

/// Table description of a `Resource`'s list view. Declares columns and how they
/// map to queries.
///
/// Row identity is mandatory and typed: [`Table::id`] declares the row-key
/// projection and [`Table::render`] errors without it, and cells render via
/// [`TextColumn`]'s lens-bound closure where typos fail at compile time instead
/// of panicking at render.
pub struct Table<M> {
    columns: Vec<TextColumn<M>>,
    filters: Vec<Filter<M>>,
    group_by: Option<GroupDef<M>>,
    row_key: Option<RowKey<M>>,
    record_key: Option<RowKey<M>>,
    row_policy: Option<RowPolicy<M>>,
    page_size: Option<usize>,
    search_ui: Option<bool>,
    filters_ui: Option<bool>,
    delete_prefix: Option<String>,
    edit_prefix: Option<String>,
    view_prefix: Option<String>,
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
            .field("record_key", &self.record_key.is_some())
            .field("row_policy", &self.row_policy.is_some())
            .field("page_size", &self.page_size)
            .field("search_ui", &self.search_ui)
            .field("filters_ui", &self.filters_ui)
            .field("delete_prefix", &self.delete_prefix)
            .field("edit_prefix", &self.edit_prefix)
            .field("view_prefix", &self.view_prefix)
            .field("bulk_delete", &self.bulk_delete)
            .field("live_search", &self.live_search)
            .finish()
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
            record_key: None,
            row_policy: None,
            page_size: None,
            search_ui: None,
            filters_ui: None,
            delete_prefix: None,
            edit_prefix: None,
            view_prefix: None,
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
    ///
    /// Display key only (GH #168): this drives keyed diffs and DOM ids —
    /// never record fetches. Action URLs and bulk values
    /// come from [`Self::pk`], which handlers resolve as the model's typed
    /// PK. The two agree in the common case (`|u| u.id.to_string()`) and
    /// diverge whenever the display projects a non-PK value.
    pub fn id(mut self, key: impl Fn(&M) -> String + Send + Sync + 'static) -> Self {
        self.row_key = Some(Arc::new(key));
        self
    }

    /// Declare the record-key projection for action URLs and bulk checkbox
    /// values (typically `|u| u.id.to_string()`), GH #168.
    ///
    /// Required before [`Self::render`] whenever action chrome is on
    /// ([`Self::with_delete`], [`Self::with_edit`], [`Self::with_view`],
    /// [`Self::with_bulk_delete`]):
    /// handlers resolve these strings as the model's typed PK (`pk_eq_expr` /
    /// `pk_in_expr` — an unparseable value 404s), so emitting a display key
    /// here 404s every delete and bulk submit. Renders with chrome but without
    /// it return an error rather than emitting keys the handlers cannot
    /// resolve.
    pub fn pk(mut self, key: impl Fn(&M) -> String + Send + Sync + 'static) -> Self {
        self.record_key = Some(Arc::new(key));
        self
    }

    /// Declare the per-record action policy (GH #235): which of the wired row
    /// actions each record may use.
    ///
    /// The `with_*` methods say which affordances the table declares; this says
    /// which of them a row may use. The renderer consults it per row — a denied
    /// action emits no link, and a row denied `delete` renders its bulk
    /// checkbox `disabled` with the reason as its accessible label, so the
    /// selection transport never carries a key the handler refuses.
    ///
    /// Defaults to [`RowActions::ALL`], so a table that declares no policy
    /// renders exactly the chrome its `with_*` calls attached. The policy is
    /// consulted only where chrome is wired: a table with no action prefix
    /// never calls it, and a resource that declares no chrome keeps its list
    /// page free of per-record predicate calls.
    ///
    /// The panel wires this from
    /// [`can_view`](crate::resource::Resource::can_view) /
    /// [`can_update`](crate::resource::Resource::can_update) /
    /// [`can_delete`](crate::resource::Resource::can_delete), each action
    /// mirroring the predicates its route checks.
    pub fn row_actions(
        mut self,
        policy: impl Fn(&M) -> RowActions + Send + Sync + 'static,
    ) -> Self {
        self.row_policy = Some(Arc::new(policy));
        self
    }

    /// Declare columns. Accepts a single column or tuple of columns.
    ///
    /// Panics on duplicate [`TextColumn::name`] (GH #156): sort resolution is
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
    ///
    /// Panics on duplicate [`Filter::name`] (GH #294), the same fail-loud
    /// policy as [`Self::columns`]: the `filters` transport is one
    /// `name:value` pair per declared filter, and `parse_filters_param` keeps
    /// the first value for a duplicated key, so two filters sharing a name
    /// would silently drop one of them.
    pub fn filters(mut self, filters: impl IntoFilters<M>) -> Self
    where
        M: toasty::schema::Model,
    {
        let filters = filters.into_filters();
        let mut seen = std::collections::HashSet::with_capacity(filters.len());
        for f in &filters {
            let name = f.name();
            assert!(
                seen.insert(name),
                "duplicate filter name '{name}': each Table filter needs a distinct name (GH #294)"
            );
        }
        self.filters = filters;
        self
    }

    /// The relations this table's columns declared their projections read,
    /// merged into one set (GH #177).
    ///
    /// The export hands this to
    /// [`Resource::export_query`](super::Resource::export_query), which is
    /// the only thing that can turn a name into a typed `include(..)`. Every
    /// column is rendered — the CSV writes a cell per column — so the set is
    /// the union over all of them; a table cannot narrow its export by
    /// declaring fewer needs than its columns read, because the reading
    /// column is the one that renders.
    pub fn include_needs(&self) -> super::IncludeNeeds
    where
        M: toasty::schema::Model,
    {
        self.columns
            .iter()
            .flat_map(|c| c.include_names())
            .copied()
            .collect()
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
    /// Documented no-op values are exempt (GH #170): `TernaryFilter`'s `all`
    /// selects no predicate by contract, so it is never flagged.
    ///
    /// An oversized `?filters=` transport arrives here as
    /// `FILTERS_OVERFLOW_SEGMENT` (GH #205), reported with its own reason so
    /// the warning says the transport was refused rather than misdescribing it
    /// as malformed.
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
                Some(f) if f.to_expr(value).is_none() && !f.is_noop_value(value) => {
                    out.push((format!("{key}:{value}"), "invalid value".to_string()))
                }
                Some(_) => {}
            }
        }
        for segment in &state.malformed_filters {
            let reason = if segment == super::state::FILTERS_OVERFLOW_SEGMENT {
                "too many filters: refused whole (GH #205)"
            } else {
                "malformed: expected key:value"
            };
            out.push((segment.clone(), reason.to_string()));
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
    ///
    /// In live tables the page-load value seeds the `group_by` interaction
    /// signal (GH #157) and persists across in-place reruns; changing it is
    /// still a navigation (`?group_by=` links) until a live control ships.
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
    /// The request entry normalizes **once** and every render seam below takes
    /// the proof ([`NormalizedState`]): the panel's list page and the
    /// `table_search` shard normalize at the point they parse the state, and
    /// pass the result down, so a live list request normalizes once instead of
    /// once per seam (GH #224). The public seams still normalize their own
    /// `&TableState` argument, so a page calling them directly keeps the GH
    /// #153 guarantee without knowing about this type.
    pub(crate) fn normalize_state(&self, state: &TableState) -> NormalizedState {
        let mut out = state.clone();
        if self.group_by.as_ref().map(|def| def.name.as_str()) != out.group_by.as_deref() {
            out.group_by = None;
        }
        NormalizedState(out)
    }

    /// Enable real cursor pagination with the given page size.
    ///
    /// Loaders pair this with toasty's `.paginate(per_page)` (via
    /// [`TablePage::from_toasty_page`]); the render then shows Previous/Next
    /// links built from the executed page's cursors — never fake page
    /// numbers. Also implies a deterministic PK ordering when the table
    /// declares no sortable column (see [`Self::order_bys_for`]).
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

    /// Which row actions `record` allows (GH #235): the declared
    /// [`Self::row_actions`] policy, or [`RowActions::ALL`] when the table
    /// declares none.
    ///
    /// The renderer reads this per row to decide the View/Edit/Delete links and
    /// whether the bulk checkbox is enabled.
    pub fn actions_for(&self, record: &M) -> RowActions {
        self.row_policy
            .as_ref()
            .map_or(RowActions::ALL, |policy| policy(record))
    }

    /// Force the search toolbar on or off.
    ///
    /// Defaults to showing the toolbar whenever at least one column is
    /// `searchable()`, so the toolbar and the query stay in step.
    pub fn search(mut self, enabled: bool) -> Self {
        self.search_ui = Some(enabled);
        self
    }

    /// Force the filter bar on or off.
    ///
    /// Defaults to showing the bar whenever the table declares filters. The
    /// live list hoists the bar out of the swapped table and turns it off here
    /// (GH #166), mirroring how `search(false)` hands the search toolbar to the
    /// page: a `<select>` that is rebuilt by its own rerun loses focus and
    /// collapses its native popup.
    pub fn filter_bar(mut self, enabled: bool) -> Self {
        self.filters_ui = Some(enabled);
        self
    }

    /// Keystroke-live search via the `table_search` shard (GH #104).
    ///
    /// When enabled, the toolbar renders a signal-backed input that
    /// re-renders the table after a short keystroke-quiet delay (GH #172,
    /// `LIVE_SEARCH_DEBOUNCE_MS`), morphing in place so focus
    /// and typing survive, instead of a GET submit. The `?q=` GET form stays
    /// inside `<noscript>` as the no-JS fallback. Opt-in per resource; the
    /// shard authorizes itself (`can_view_any` + the tenant-scoped query,
    /// GH #223) and every arg is validated like the GET path.
    /// Per-row `can_view` is not applied here, matching the list page:
    /// page-local row filtering would mislabel pagination, so row scoping
    /// belongs in `Resource::query`, inside that scope (GH #86).
    /// Note: Topcoat coalesces same-tick keystrokes and aborts in-flight
    /// reruns (latest wins); the time-based debounce above composes with
    /// that (delayed writes rerun normally).
    pub fn live_search(mut self, enabled: bool) -> Self {
        self.live_search = enabled;
        self
    }

    /// Enable row-level `Delete` action. When set, each row renders a
    /// `Delete` button that POSTs to `{prefix}/{id}/delete` with
    /// `requires_confirmation` semantics. `{id}` is the [`Self::pk`]
    /// record key (handlers resolve it as the typed PK) — rendering with
    /// delete chrome but no `pk` is a render error (GH #168).
    ///
    /// The action is gated per record by [`Self::row_actions`] (GH #235): a row
    /// the policy denies renders no `Delete` link and a disabled bulk checkbox,
    /// matching the handler's `can_view` + `can_delete` check.
    pub fn with_delete(mut self, prefix: String) -> Self {
        self.delete_prefix = Some(prefix);
        self
    }

    /// Enable row-level `Edit` action (GH #162). When set, each row renders
    /// an `Edit` link to `{prefix}/{id}/edit` (Filament's `recordActions`
    /// `EditAction`, same last-column slot as `Delete`). `{id}` is the
    /// [`Self::pk`] record key — rendering with edit chrome but no `pk` is a
    /// render error (GH #168).
    ///
    /// The action is gated per record by [`Self::row_actions`] (GH #235): a row
    /// the policy denies renders no `Edit` link, matching the edit route's
    /// `can_view` + `can_update` check. The list still renders every row —
    /// `can_view` stays out of the query, so pagination is not mislabelled
    /// (GH #86).
    pub fn with_edit(mut self, prefix: String) -> Self {
        self.edit_prefix = Some(prefix);
        self
    }

    /// Enable the row-level `View` action (GH #187). When set, each row renders
    /// a `View` link to `{prefix}/{id}` — the detail page — in the same
    /// last-column slot as `Edit` and `Delete`. `{id}` is the [`Self::pk`]
    /// record key, like the edit URL.
    ///
    /// The caller sets this only for a resource that declares a detail page
    /// ([`Resource::viewed`](crate::resource::Resource::viewed)), so a resource
    /// with no view renders no link instead of one that 404s. The action is
    /// gated per record by [`Self::row_actions`] (GH #235): a row the policy
    /// denies renders no `View` link, matching the detail route's `can_view`.
    pub fn with_view(mut self, prefix: String) -> Self {
        self.view_prefix = Some(prefix);
        self
    }

    /// Enable bulk selection with `BulkDelete` action. Checkbox values are
    /// the [`Self::pk`] record keys (handlers resolve them as typed PKs) —
    /// rendering with bulk chrome but no `pk` is a render error (GH #168).
    ///
    /// A row the [`Self::row_actions`] policy denies `delete` renders its
    /// checkbox `disabled` with the reason as its accessible label (GH #235),
    /// so select-all never submits a batch the handler's all-or-nothing check
    /// refuses.
    pub fn with_bulk_delete(mut self, enabled: bool) -> Self {
        self.bulk_delete = enabled;
        self
    }

    /// Whether the bulk checkbox column renders: bulk selection plus a delete
    /// prefix to post to (GH #74).
    fn bulk_enabled(&self) -> bool {
        self.bulk_delete && self.delete_prefix.is_some()
    }

    /// Global search predicate — OR across searchable columns.
    ///
    /// Substring match (`?q=` anywhere in the value), escaped so a term
    /// containing `%` or `_` stays literal (GH #116); see
    /// [`TextColumn::to_search_expr`](crate::resource::TextColumn::to_search_expr)
    /// for the driver case-sensitivity caveat.
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
    /// app-level tie-breaker: toasty's engine appends the physical PK columns
    /// to ambiguous cursor orderings internally (`normalize_cursor_order`,
    /// tokio-rs/toasty#1142), so page contents are deterministic on SQL
    /// backends without Argentum's help (GH #76).
    pub fn order_by(&self, descending: bool) -> Option<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        self.columns.iter().find_map(|c| c.to_order_by(descending))
    }

    /// Order-bys over the model's primary key (asc, in declared order) —
    /// built through the public facade (`Model::path_field` + `Path::asc`),
    /// no `toasty_core` needed.
    ///
    /// Used only when a table declares no sortable column at all: toasty's
    /// planner requires an `ORDER BY` for cursor pagination and its
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

    /// Resolve the full query ordering for a request (GH #210).
    ///
    /// Single source of truth for loaders, render and the export:
    /// 1. `?sort=<column>&dir=asc|desc` when `<column>` names a declared sortable column — that
    ///    column's direction (toasty appends the PK tie-breakers internally, see
    ///    [`Self::order_by`]);
    /// 2. otherwise the declared default (first sortable column asc);
    /// 3. otherwise, when `mode` asks for it, the PK alone — cursor pagination requires a
    ///    deterministic order even with no sortable column, and toasty only *extends* an existing
    ///    non-empty ordering.
    ///
    /// Loaders that also need the search term parse the state once with
    /// [`TableState::from_cx`] and apply the declaration through
    /// `Self::apply_declaration` (see `crate::panel::Panel`'s generic
    /// resource list handler).
    pub fn order_bys_for(&self, state: &TableState, mode: OrderMode) -> Vec<OrderByExpr>
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
        let out: Vec<OrderByExpr> = self.order_by(false).into_iter().collect();
        if out.is_empty() && mode.falls_back_to_pk(self.page_size.is_some()) {
            return Self::pk_order_bys();
        }
        out
    }

    /// Apply this table's declaration to `query` — the one routine that turns
    /// the search term, the filters and the ordering into a query (GH #210).
    ///
    /// `query` is the caller's seed, which is the one thing the two loaders
    /// legitimately differ on: the list loads the tenant-scoped
    /// [`Resource::query`](crate::resource::Resource::query) (the row-scoping
    /// seam, ADR-0002) while the export loads the tenant-scoped
    /// [`Resource::export_query`](crate::resource::Resource::export_query),
    /// narrowed to the relations the rendered columns declared (GH #177).
    /// `mode` picks the ordering fallback each caller needs.
    ///
    /// Everything else is shared, so a new search or filter dimension cannot
    /// reach the list and miss the CSV — the drift class GH #172 fixed.
    pub(crate) fn apply_declaration(
        &self,
        mut query: toasty::stmt::Query<List<M>>,
        state: &TableState,
        mode: OrderMode,
    ) -> toasty::stmt::Query<List<M>>
    where
        M: toasty::schema::Model,
    {
        if let Some(term) = &state.search
            && let Some(expr) = self.search_expr(term)
        {
            query = query.filter(expr);
        }
        if let Some(expr) = self.filter_expr(state) {
            query = query.filter(expr);
        }
        for ord in self.order_bys_for(state, mode) {
            query = query.order_by(ord);
        }
        query
    }

    /// Resolve and execute this table's query for `state` — search, filters,
    /// ordering, and cursor pagination — and return the rows.
    ///
    /// The loader half of the live-table seam (GH #154 §2): a page that owns
    /// its own table (the showcase demos) can hand its shard a query and this
    /// hook applies the same declaration pipeline `panel::load_table_page`
    /// applies to the tenant-scoped `Resource::query` (GH #223), so a
    /// page-level shard does not
    /// reimplement filtering, ordering, or cursor validation.
    ///
    /// The cursor-existence probes reuse `query`, so they pay the query's
    /// relation includes. The panel's resource-list loader seeds them from a
    /// narrower query with the same scope and no includes; this entry point has
    /// no such seed, so its probes carry the query's includes.
    pub async fn load(
        &self,
        cx: &Cx,
        query: toasty::stmt::Query<List<M>>,
        state: &TableState,
    ) -> Result<TablePage<M>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        self.load_with_probe(cx, query.clone(), query, state).await
    }

    /// [`Self::load`] with a separate seed for the cursor-existence probes
    /// (GH #298).
    ///
    /// The probes only ask whether one more row exists past a cursor, so they
    /// read no relation and do not need `query`'s includes. `probe_query` is
    /// the same scope and declaration pipeline with those includes dropped;
    /// passing `query` itself reproduces [`Self::load`].
    pub(crate) async fn load_with_probe(
        &self,
        cx: &Cx,
        query: toasty::stmt::Query<List<M>>,
        probe_query: toasty::stmt::Query<List<M>>,
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
        // The declaration becomes predicates and an ordering through the one
        // shared routine (GH #210) — the export loader applies the same one to
        // its own seed query.
        let query = self.apply_declaration(query, state, OrderMode::List);
        let mut db = crate::db::db(cx);
        match self.page_size {
            Some(per_page) => {
                // Keep a cursor-free copy of the filtered+ordered probe seed for
                // cursor validation: Toasty's `Page` sets `next_cursor`
                // optimistically whenever `len == page_size`, which leaves a
                // phantom cursor when the page sits exactly at a boundary. The
                // probe seed carries no relation includes (GH #298) because the
                // probes only ask whether a row exists.
                let base_query = self.apply_declaration(probe_query, state, OrderMode::List);
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
                    .map_err(|error| reject_cursor(error.into(), state))?;
                let mut page = TablePage::from_toasty_page(loaded)?;
                // Cursor-existence probes, one per landing direction (GH #172):
                // the engine sets `next_cursor`/`prev_cursor` optimistically,
                // so a page sitting exactly at a boundary carries a phantom
                // cursor without validation. Each direction probes only the
                // edge that can lie:
                // - forward/first landing: prev is exact (absent on the first page; otherwise the
                //   page we came from exists), next may be phantom at the end boundary → probe next
                //   on full pages. A short page cannot have a next page (GH #75).
                // - backward landing: next is exact (the page we came from follows), prev may be
                //   phantom when the fetch lands on the first page → probe prev whenever one is
                //   reported.
                //
                // Deliberately NOT a `LIMIT per_page+1` fold: the engine
                // derives `next_cursor` from the last *fetched* row, so
                // trimming the extra row would anchor the next link past it —
                // every `(per_page+1)`th row would vanish from forward walks.
                // The probes keep the main fetch's cursors (which point at
                // displayed rows) as the link anchors.
                //
                // Residual (same as ever): a concurrent delete landing between
                // the main fetch and the click can still void a validated
                // cursor — that degrades to the void-window recovery link
                // (GH #98), never to silently skipped rows.
                if state.before.is_some() {
                    if let Some(cursor) = page.prev_cursor.clone() {
                        let probe = toasty::stmt::Paginate::new(base_query, 1)
                            .before(crate::cursor::decode(&cursor)?)
                            .exec(&mut db)
                            .await
                            .map_err(topcoat::Error::from)?;
                        if probe.items.is_empty() {
                            page.prev_cursor = None;
                        }
                    }
                } else if page.rows.len() == per_page {
                    if let Some(cursor) = page.next_cursor.clone() {
                        let probe = toasty::stmt::Paginate::new(base_query, 1)
                            .after(crate::cursor::decode(&cursor)?)
                            .exec(&mut db)
                            .await
                            .map_err(topcoat::Error::from)?;
                        if probe.items.is_empty() {
                            page.next_cursor = None;
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

    /// The first declaration this table is missing, if any (GH #138).
    ///
    /// The same checks [`Self::render`](Self::render_with_state) enforces per
    /// request, lifted so [`Panel::build`](crate::panel::Panel::build) can
    /// refuse to serve a resource whose table could never render — the
    /// declaration is knowable at boot, so a request is too late to report it.
    ///
    /// `chrome` is the action chrome the caller will attach (GH #207): the
    /// declared table carries no delete/edit/view prefix, so the record-key
    /// requirement is only knowable once the wiring is known. The render
    /// enforces the same predicate on the wired table.
    pub(crate) fn missing_essentials(&self, chrome: TableChrome) -> Option<String> {
        if self.page_size == Some(0) {
            return Some("paginate requires per_page > 0".to_string());
        }
        if self.columns.is_empty() {
            return Some(
                "no columns declared — declare columns via Table::columns(..)".to_string(),
            );
        }
        if self.row_key.is_none() {
            return Some("no row key declared — declare one via Table::id(|row| ..)".to_string());
        }
        if chrome.actions() && self.record_key.is_none() {
            return Some(
                "action chrome needs a record key — declare one via Table::pk(|row| ..)"
                    .to_string(),
            );
        }
        None
    }

    /// Whether the search toolbar renders: the explicit `search(bool)` value,
    /// or auto — at least one `searchable()` column.
    pub(crate) fn search_enabled(&self) -> bool
    where
        M: toasty::schema::Model,
    {
        self.search_ui
            .unwrap_or_else(|| self.columns.iter().any(|c| c.is_searchable()))
    }

    /// Whether this table renders the keystroke-live search host (GH #104).
    pub(crate) fn is_live_search(&self) -> bool {
        self.live_search
    }

    /// Whether the filter bar renders inside the table: the explicit
    /// `filter_bar(bool)` value, or auto — the table declares at least one filter.
    ///
    /// Live tables turn it off (GH #166): the list page hoists the bar out of
    /// the swapped region, the same way it owns the search toolbar, so a filter
    /// change cannot rebuild the control the user is interacting with.
    pub(crate) fn filter_bar_enabled(&self) -> bool {
        self.filters_ui.unwrap_or(!self.filters.is_empty())
    }
}

/// Attribute a failed paginated fetch to the request's cursor (GH #294).
///
/// A token cut from a different ordering decodes but the engine refuses the
/// statement (`invalid_statement`: its field count no longer matches the
/// query's `ORDER BY`). No other statement this paginated loader builds carries
/// that error while the request names a cursor. Such a failure is the cursor's,
/// so it takes the cursor-stripped retry contract instead of re-requesting the
/// identical URL forever; every other failure keeps the cursor (GH #98).
fn reject_cursor(error: topcoat::Error, state: &TableState) -> topcoat::Error {
    let cursored = state.after.is_some() || state.before.is_some();
    let rejected = error
        .downcast_ref::<toasty::Error>()
        .is_some_and(toasty::Error::is_invalid_statement);
    if cursored && rejected {
        crate::cursor::CursorRejectedError::rejected(&error)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use toasty::{Db, stmt::List};
    use topcoat::context::CxTestBuilder;

    use super::*;
    use crate::resource::{SelectFilter, Sort, TableState, TernaryFilter, TextColumn};

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

    fn status_table(cx: &Cx) -> Table<Task> {
        Table::<Task>::r#for(cx)
            .id(|t| t.id.to_string())
            .pk(|t| t.id.to_string())
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
            .pk(|u| u.id.to_string())
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

    /// GH #177: the export asks its table which relations the rendered
    /// columns declared; the union across columns is that answer, and a table
    /// whose columns read no relation declares nothing (so the resource's
    /// `export_query` can drop every include).
    #[test]
    fn table_include_needs_unions_the_columns_declarations() {
        let cx = CxTestBuilder::new().build();
        let plain = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(plain.include_needs().is_empty());

        let declared = Table::<Task>::r#for(&cx).columns((
            TextColumn::r#for(Task::fields().title(), |t| t.title.clone()),
            TextColumn::computed("Owner", |t: &Task| t.title.clone()).needs(["author"]),
            TextColumn::computed("Audit", |t: &Task| t.title.clone()).needs(["comments", "author"]),
        ));
        let needs = declared.include_needs();
        assert!(needs.wants("author") && needs.wants("comments"));
        // Only what a column declared: `author` declared twice is still a
        // member, and an undeclared name is absent.
        assert!(!needs.wants("tenant"));
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
        let orders = users_table.order_bys_for(&TableState::default(), OrderMode::List);
        // Single sortable column, no app-level PK suffix — toasty's engine
        // appends the physical PK columns to ambiguous cursor orderings
        // internally (GH #76).
        assert_eq!(orders.len(), 1, "sortable column only, got {orders:?}");
        // No sortable → empty
        let table_none = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(
            table_none
                .order_bys_for(&TableState::default(), OrderMode::List)
                .is_empty(),
            "non-sortable should have no order_bys"
        );
    }

    #[test]
    fn order_bys_for_resolves_sort_param_with_fallbacks() {
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
        let orders = sorted.order_bys_for(&state, OrderMode::List);
        assert_eq!(orders.len(), 1, "sort column only, got {orders:?}");

        // Unknown sort column → declared default (name asc)
        let state = TableState {
            sort: Some(Sort {
                column: "nope".to_string(),
                descending: false,
            }),
            ..TableState::default()
        };
        assert_eq!(sorted.order_bys_for(&state, OrderMode::List).len(), 1);

        // No sort at all → declared default
        assert_eq!(
            sorted
                .order_bys_for(&TableState::default(), OrderMode::List)
                .len(),
            1
        );

        // Paginated table with no sortable column → PK-only deterministic order
        let unsorted = Table::<User>::r#for(&cx)
            .paginate(25)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let orders = unsorted.order_bys_for(&TableState::default(), OrderMode::List);
        assert_eq!(
            orders.len(),
            1,
            "PK-only for paginated unsorted, got {orders:?}"
        );

        // Unpaginated and unsorted → empty (query stays unordered)
        let plain = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        assert!(
            plain
                .order_bys_for(&TableState::default(), OrderMode::List)
                .is_empty()
        );

        // The export mode pins an unordered table to PK order (GH #172):
        // the chunked cursor walk needs a deterministic order whether or not
        // the table paginates.
        assert_eq!(
            plain
                .order_bys_for(&TableState::default(), OrderMode::Export)
                .len(),
            1,
            "export mode must fall back to the PK for an unordered table"
        );
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
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());
        let mut db = crate::db::db(&cx);

        // Page 1 of 1-per-page: full page → real next cursor.
        let page1 = users_table
            .order_bys_for(&TableState::default(), OrderMode::List)
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

    #[tokio::test]
    async fn table_renders_inside_the_boundary_region() {
        // GH #136: relocated from the showcase (`table_boundary_and_memoize`)
        // — core owns the boundary contract; the showcase owns HTTP wiring.
        // The topcoat `#[memoize]` half stays upstream and is not re-pinned
        // here. The region is unconditional since GH #220 dropped the
        // `boundary(..)` opt-out (it had no caller), so the table always lands
        // where a morph can swap it.
        use topcoat::view::ViewExt;

        let cx = CxTestBuilder::new().build();
        let table = Table::<User>::r#for(&cx)
            .id(|u: &User| u.id.to_string())
            .pk(|u: &User| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u: &User| {
                u.name.clone()
            }));

        let page = crate::resource::TablePage::<User>::from(vec![]);
        let html = table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let boundary_at = html.find("data-boundary=\"table\"").unwrap_or_else(|| {
            panic!("the table must render inside the morph boundary, got {html}")
        });
        let root_at = html
            .find("data-table-root=\"\"")
            .unwrap_or_else(|| panic!("the swapped table must carry its table root, got {html}"));
        assert!(
            boundary_at < root_at && !html[boundary_at..root_at].contains("</div>"),
            "the boundary must wrap the table root, got {html}"
        );
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

    #[test]
    fn unapplied_filters_flags_a_refused_filters_transport() {
        // GH #205: an oversized `?filters=` is refused whole rather than
        // partially applied, and it reads as its own reason — not as a
        // malformed segment — so the list banner explains itself and the
        // export's 400 is the fail-closed guard instead of a silent drop.
        let cx = CxTestBuilder::new().build();
        let tbl = status_table(&cx);
        let huge = format!("status:published,{}", "k:v,".repeat(2 * 1024 * 1024));
        let state = TableState::from_live_args("", &huge, "", "", "");
        assert!(
            state.filters.is_empty() && tbl.filter_expr(&state).is_none(),
            "the refused transport must apply no predicate"
        );
        assert_eq!(
            tbl.unapplied_filters(&state),
            vec![(
                "filters=overflow".to_string(),
                "too many filters: refused whole (GH #205)".to_string()
            )]
        );
    }

    #[test]
    fn ternary_all_is_a_neutral_noop_not_an_invalid_value() {
        // GH #170: `all` is the documented TernaryFilter no-op — it selects
        // no predicate AND is never flagged, so the list shows no warning
        // and the export (which refuses on any unapplied filter) stays 200.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .pk(|t| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t| {
                t.title.clone()
            }))
            .filters(TernaryFilter::r#for(Task::fields().featured()));
        let state = filters_state(&[("featured", "all")]);
        assert!(
            tbl.filter_expr(&state).is_none(),
            "all must select no predicate"
        );
        assert!(
            tbl.unapplied_filters(&state).is_empty(),
            "all must not be flagged, got {:?}",
            tbl.unapplied_filters(&state)
        );
        // Genuine garbage still flags.
        assert_eq!(
            tbl.unapplied_filters(&filters_state(&[("featured", "maybe")])),
            vec![("featured:maybe".to_string(), "invalid value".to_string())]
        );
    }

    #[tokio::test]
    async fn filter_banner_reports_unfiltered_when_nothing_applies() {
        // GH #170: an invalid-only request applies no predicate, so the
        // banner must say "showing unfiltered results" — "other filter(s)
        // still apply" would be the lie. Mixed valid+invalid keeps the old
        // tail (GH #148).
        use topcoat::view::ViewExt;
        let cx = CxTestBuilder::new().build();
        let tbl = status_table(&cx);
        let render_banner = async |pairs: &[(&str, &str)]| {
            let page = crate::resource::TablePage::<Task>::from(vec![]);
            tbl.render_with_state(&cx, page, &filters_state(pairs), "/admin/tasks")
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(&cx)
        };
        let html = render_banner(&[("status", "typo")]).await;
        assert!(
            html.contains("showing unfiltered results"),
            "invalid-only banner must admit unfiltered, got {html}"
        );
        let html = render_banner(&[("status", "published"), ("bogus", "x")]).await;
        assert!(
            html.contains("other filter(s) still apply"),
            "mixed banner keeps the GH #148 tail, got {html}"
        );
    }

    #[test]
    #[should_panic(expected = "duplicate column name")]
    fn duplicate_column_name_panics_on_field_computed_collision() {
        // GH #156: computed("Status") derives name "status", colliding with
        // the field column's name — the TextColumn::name namespace must stay
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
    #[should_panic(expected = "duplicate filter name")]
    fn duplicate_filter_name_panics_on_duplicate_field() {
        // GH #294: the transport names a filter by its field, and the parser
        // keeps the first value for a duplicated key, so two filters on one
        // field would silently drop one. Refuse the declaration instead.
        let _ = Table::<Task>::new().filters((
            SelectFilter::r#for(Task::fields().status(), vec!["published".to_string()]),
            SelectFilter::r#for(Task::fields().status(), vec!["draft".to_string()]),
        ));
    }

    async fn seeded_users(names: &[&str]) -> topcoat::context::Cx {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in names {
            toasty::create!(User {
                name: name.to_string()
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        CxTestBuilder::new().app_context(db).build()
    }

    fn paged_users_table(cx: &topcoat::context::Cx, per_page: usize) -> Table<User> {
        Table::<User>::r#for(cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone()).sortable())
            .paginate(per_page)
    }

    #[tokio::test]
    async fn full_walk_reaches_every_row_exactly_once_without_phantoms() {
        // GH #172: prev/next existence must be exact at every boundary — no
        // phantom links to empty pages, and no skipped rows. A `LIMIT
        // per_page+1` fold with the extra row trimmed would anchor the next
        // link past the extra row (the engine derives cursors from the last
        // *fetched* row), dropping every `(per_page+1)`th row from forward
        // walks — this walk fails loudly if that ever lands.
        let cx = seeded_users(&["u01", "u02", "u03", "u04", "u05"]).await;
        let tbl = paged_users_table(&cx, 2);
        let query = || toasty::stmt::Query::<List<User>>::all();
        // Forward walk from the first page to exhaustion.
        let mut seen = Vec::new();
        let mut state = TableState::default();
        let mut last = tbl.load(&cx, query(), &state).await.unwrap();
        assert!(last.prev_cursor.is_none(), "first page has no prev");
        loop {
            seen.extend(last.rows.iter().map(|u| u.name.clone()));
            match last.next_cursor.clone() {
                Some(cursor) => {
                    state = TableState {
                        after: Some(cursor),
                        ..TableState::default()
                    };
                    last = tbl.load(&cx, query(), &state).await.unwrap();
                }
                None => break,
            }
        }
        assert_eq!(seen, vec!["u01", "u02", "u03", "u04", "u05"]);
        // Backward walk from the terminal page to the first.
        let mut back = vec![last.rows.iter().map(|u| u.name.clone()).collect::<Vec<_>>()];
        while let Some(cursor) = last.prev_cursor.clone() {
            state = TableState {
                before: Some(cursor),
                ..TableState::default()
            };
            last = tbl.load(&cx, query(), &state).await.unwrap();
            back.push(last.rows.iter().map(|u| u.name.clone()).collect::<Vec<_>>());
        }
        back.reverse();
        assert_eq!(
            back,
            vec![
                vec!["u01".to_string(), "u02".to_string()],
                vec!["u03".to_string(), "u04".to_string()],
                vec!["u05".to_string()],
            ]
        );
        assert!(last.prev_cursor.is_none(), "first page has no prev");
    }

    #[tokio::test]
    async fn exact_boundary_pages_carry_exact_cursors() {
        // GH #172: a full page sitting exactly at the boundary (4 rows,
        // `paginate(2)`) must report no next page — the engine's optimistic
        // `next_cursor` alone would be a phantom link to an empty page.
        let cx = seeded_users(&["u01", "u02", "u03", "u04"]).await;
        let tbl = paged_users_table(&cx, 2);
        let query = || toasty::stmt::Query::<List<User>>::all();
        let first = tbl
            .load(&cx, query(), &TableState::default())
            .await
            .unwrap();
        assert_eq!(first.rows.len(), 2);
        let cursor = first.next_cursor.clone().expect("page 1 of 2 has a next");
        let state = TableState {
            after: Some(cursor),
            ..TableState::default()
        };
        let second = tbl.load(&cx, query(), &state).await.unwrap();
        assert_eq!(
            second
                .rows
                .iter()
                .map(|u| u.name.clone())
                .collect::<Vec<_>>(),
            vec!["u03".to_string(), "u04".to_string()]
        );
        assert!(
            second.next_cursor.is_none(),
            "terminal full page must not offer a next page"
        );
        assert!(
            second.prev_cursor.is_some(),
            "second page must offer a prev page"
        );
    }

    /// Counts sqlite driver executions inside the `gh172-budget` marker span.
    /// Tracing caches per-callsite interest globally at first use: a sibling
    /// test executing first pins the driver's callsite as `never`, after
    /// which no thread-local subscriber can observe it. So the budget test
    /// installs this as the *global* default once (registration then sticks
    /// at `always`) and attributes execs by span — sibling tests' execs fall
    /// outside the marker span and are ignored.
    struct BudgetState {
        count: std::sync::atomic::AtomicUsize,
        next_span: std::sync::atomic::AtomicU64,
    }

    /// Marker span attributing driver execs to the budget measurement.
    const BUDGET_SPAN: &str = "gh172-budget";

    thread_local! {
        static BUDGET_MARKERS: std::cell::RefCell<std::collections::HashSet<u64>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
        static BUDGET_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    struct BudgetVisitor {
        driver: Option<String>,
        message: String,
    }

    impl tracing::field::Visit for BudgetVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "driver" {
                self.driver = Some(value.to_string());
            }
            self.record_debug(field, &format_args!("{value}"));
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{value:?}");
            }
        }
    }

    impl tracing::Subscriber for BudgetState {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.target().starts_with("toasty_driver_sqlite")
                || (metadata.is_span() && metadata.name() == BUDGET_SPAN)
        }

        fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            let id = self
                .next_span
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if span.metadata().name() == BUDGET_SPAN {
                BUDGET_MARKERS.with(|markers| {
                    markers.borrow_mut().insert(id);
                });
            }
            tracing::span::Id::from_u64(id)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let in_scope = BUDGET_DEPTH.with(|depth| depth.get() > 0);
            if !in_scope {
                return;
            }
            let mut visitor = BudgetVisitor {
                driver: None,
                message: String::new(),
            };
            event.record(&mut visitor);
            if visitor.driver.as_deref() == Some("sqlite")
                && visitor.message.contains("driver exec")
            {
                self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        fn enter(&self, span: &tracing::span::Id) {
            let is_marker =
                BUDGET_MARKERS.with(|markers| markers.borrow().contains(&span.into_u64()));
            if is_marker {
                BUDGET_DEPTH.with(|depth| depth.set(depth.get() + 1));
            }
        }

        fn exit(&self, span: &tracing::span::Id) {
            let is_marker =
                BUDGET_MARKERS.with(|markers| markers.borrow().contains(&span.into_u64()));
            if is_marker {
                BUDGET_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
            }
        }
    }

    static BUDGET_INSTALL: std::sync::OnceLock<std::sync::Arc<BudgetState>> =
        std::sync::OnceLock::new();

    /// Install the budget counter as the process-global default (once) and
    /// hand back its handle. Later interest-cache state cannot regress: no
    /// other subscriber exists in this binary, so the driver's callsite stays
    /// `always` from here on.
    fn install_budget_counter() -> std::sync::Arc<BudgetState> {
        BUDGET_INSTALL
            .get_or_init(|| {
                let state = std::sync::Arc::new(BudgetState {
                    count: std::sync::atomic::AtomicUsize::new(0),
                    next_span: std::sync::atomic::AtomicU64::new(1),
                });
                tracing::subscriber::set_global_default(state.clone())
                    .expect("budget counter installs once");
                tracing::callsite::rebuild_interest_cache();
                state
            })
            .clone()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn full_page_costs_main_plus_single_direction_probe() {
        // GH #172: a full page costs the main fetch plus exactly one `LIMIT
        // 1` existence probe — next on forward/first landings, prev on
        // backward landings (each direction probes only the edge that can
        // lie). A short forward page costs the main fetch alone. Counts are
        // calibrated in-test against bare toasty execs, so no
        // engine-internal constant is pinned. `current_thread`: the marker
        // span is entered and polled on one thread (no hops), so the
        // thread-local attribution below holds.
        use std::sync::atomic::Ordering;
        let cx = seeded_users(&["u01", "u02", "u03", "u04"]).await;
        let tbl = paged_users_table(&cx, 2);
        let budget = install_budget_counter();
        let _scope = tracing::info_span!(BUDGET_SPAN).entered();
        let count_around = |reset: bool| {
            if reset {
                budget.count.store(0, Ordering::SeqCst);
            }
            budget.count.load(Ordering::SeqCst)
        };
        let ordered = || User::all().order_by(User::fields().name().asc());
        let mut db = crate::db::db(&cx);
        // Baselines: one bare main-shaped exec and one bare probe-shaped exec.
        count_around(true);
        let bare_main = ordered().paginate(2).exec(&mut db).await.unwrap();
        let bare_main_cost = count_around(false);
        count_around(true);
        let probe_cursor = bare_main.next_cursor.clone().unwrap();
        ordered()
            .paginate(1)
            .after(probe_cursor)
            .exec(&mut db)
            .await
            .unwrap();
        let bare_probe_cost = count_around(false);
        assert!(
            bare_main_cost > 0 && bare_probe_cost > 0,
            "the counter must observe driver execs, got main={bare_main_cost} probe={bare_probe_cost}"
        );
        // Full first page: main + exactly one next probe.
        count_around(true);
        let first = tbl
            .load(
                &cx,
                toasty::stmt::Query::<List<User>>::all(),
                &TableState::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            count_around(false),
            bare_main_cost + bare_probe_cost,
            "full page must cost exactly main + one next probe"
        );
        assert_eq!(first.rows.len(), 2);
        // Short terminal page: main alone, no probe (paginate(3) over 4
        // rows ends on a 1-row page).
        let tbl3 = paged_users_table(&cx, 3);
        count_around(true);
        let head = tbl3
            .load(
                &cx,
                toasty::stmt::Query::<List<User>>::all(),
                &TableState::default(),
            )
            .await
            .unwrap();
        assert_eq!(head.rows.len(), 3);
        let tail_state = TableState {
            after: head.next_cursor.clone(),
            ..TableState::default()
        };
        count_around(true);
        let tail = tbl3
            .load(&cx, toasty::stmt::Query::<List<User>>::all(), &tail_state)
            .await
            .unwrap();
        assert_eq!(tail.rows.len(), 1);
        assert!(tail.next_cursor.is_none());
        let short_cost = count_around(false);
        count_around(true);
        ordered().paginate(3).exec(&mut db).await.unwrap();
        let bare_short_cost = count_around(false);
        assert_eq!(
            short_cost, bare_short_cost,
            "short page must cost exactly one bare fetch (no probe)"
        );
        // Backward landing on a full page: main + exactly one prev probe
        // (pp=2 table: page 2 [u03,u04], then back to full page 1).
        let p1 = tbl
            .load(
                &cx,
                toasty::stmt::Query::<List<User>>::all(),
                &TableState::default(),
            )
            .await
            .unwrap();
        let p2_state = TableState {
            after: p1.next_cursor.clone(),
            ..TableState::default()
        };
        let p2 = tbl
            .load(&cx, toasty::stmt::Query::<List<User>>::all(), &p2_state)
            .await
            .unwrap();
        assert_eq!(p2.rows.len(), 2);
        let back_to_first = TableState {
            before: p2.prev_cursor.clone(),
            ..TableState::default()
        };
        count_around(true);
        let first_again = tbl
            .load(
                &cx,
                toasty::stmt::Query::<List<User>>::all(),
                &back_to_first,
            )
            .await
            .unwrap();
        assert_eq!(
            first_again
                .rows
                .iter()
                .map(|u| u.name.clone())
                .collect::<Vec<_>>(),
            vec!["u01".to_string(), "u02".to_string()]
        );
        assert!(
            first_again.prev_cursor.is_none(),
            "backward landing on the first page must hide the phantom prev"
        );
        assert_eq!(
            count_around(false),
            bare_main_cost + bare_probe_cost,
            "backward landing must cost exactly main + one prev probe"
        );
    }

    #[tokio::test]
    async fn stale_cursor_is_marked_for_retry() {
        // GH #294: a token cut from another ordering decodes but the engine
        // refuses the statement (the cursor's field count no longer matches
        // the query's `ORDER BY`). That failure is the cursor's, so it carries
        // a cursor marker and the retry drops pagination instead of repeating
        // the identical failing request forever.
        use toasty::stmt::Value;
        use toasty_core::stmt::ValueRecord;

        let mut db = Db::builder()
            .models(toasty::models!(Task))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for title in ["Alpha", "Bravo", "Charlie", "Delta"] {
            toasty::create!(Task {
                title: title.to_string(),
                status: "draft".to_string(),
                featured: false,
                created_at: "2024-01-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = topcoat::context::CxTestBuilder::new()
            .app_context(db)
            .build();
        let table = || {
            Table::<Task>::new()
                .id(|t| t.id.to_string())
                .pk(|t| t.id.to_string())
                .columns(
                    TextColumn::r#for(Task::fields().title(), |t: &Task| t.title.clone())
                        .sortable(),
                )
                .paginate(2)
        };
        // The query orders by `title` then the PK to break ties, so a cursor
        // with three fields has one too many.
        let wide = crate::cursor::encode(&Value::Record(ValueRecord::from_vec(vec![
            Value::String("Alpha".to_string()),
            Value::String("x".to_string()),
            Value::I64(1),
        ])))
        .unwrap();
        let state = TableState {
            after: Some(wide),
            ..TableState::default()
        };
        let error = table()
            .load(&cx, toasty::stmt::Query::<List<Task>>::all(), &state)
            .await
            .expect_err("a cursor with too many fields must fail the load");
        assert!(
            crate::cursor::is_cursor_error(&error),
            "a rejected cursor must carry the cursor marker, got {error}"
        );
        assert!(
            error
                .downcast_ref::<crate::cursor::CursorRejectedError>()
                .is_some(),
            "the refusal is not a decode failure, got {error}"
        );

        // A transient failure keeps the cursor (GH #98): a failure the cursor
        // did not cause carries no marker, so `retry_url_for_error` keeps the
        // pagination it was given.
        let transient = topcoat::Error::from(std::io::Error::other("database unavailable"));
        assert!(
            !crate::cursor::is_cursor_error(&transient),
            "only cursor failures drop pagination on retry"
        );

        // A cursor cut from this query's own ordering round-trips: the guard
        // marks a rejected cursor, not every request that carries one.
        let first = table()
            .load(
                &cx,
                toasty::stmt::Query::<List<Task>>::all(),
                &TableState::default(),
            )
            .await
            .unwrap();
        let state = TableState {
            after: first.next_cursor.clone(),
            ..TableState::default()
        };
        assert!(
            table()
                .load(&cx, toasty::stmt::Query::<List<Task>>::all(), &state)
                .await
                .is_ok(),
            "a matching cursor must keep loading"
        );
    }
}
