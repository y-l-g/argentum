//! Cursor/URL state: [`TablePage`], [`Sort`], [`TableState`], and the URL codec.
//!
//! Both entry points share one parse contract (GH #206) and the `filters`
//! transport is bounded where it is parsed (GH #205).

use std::collections::HashMap;

use topcoat::{
    Result,
    context::Cx,
    runtime::{Signal, signal},
};

use crate::query_term::clamp_query_term;

/// The live table's browser state (GH #151).
///
/// The page owns these signals and hands their handles to the `table_search`
/// shard through `Table::render_live_with_state`; each tracked read inside
/// the shard becomes a `dep` marker the browser watches, so writing any signal
/// re-renders the table in place — no navigation, no scroll jump. Sort links,
/// the pager, the filter transport, the bulk selection, and the clear links
/// rendered by the table write them.
///
/// The two carriers meet in exactly two methods (GH #224):
/// [`TableState::to_signals`] seeds these handles from a parsed state, and
/// [`TableSignals::to_state`] rebuilds the state from their current values.
/// A new interaction dimension is a field here plus one arm in each, instead
/// of a hand-written conversion at every seam.
///
/// `q`/`filters`/`sort`/`dir`/`group_by` reset the cursor when they change;
/// [`Self::cursor`] pages within the current result set. All values are
/// untrusted by the time the shard reads them back (the client owns the
/// signal).
#[derive(Clone)]
pub struct TableSignals {
    /// `?q=` — the search term (escaped substring match, GH #116).
    pub q: Signal<String>,
    /// `?filters=` — the composed `key:value,key2:value2` transport.
    pub filters: Signal<String>,
    /// `?sort=` — the active sort column name (`""` = the table default).
    pub sort: Signal<String>,
    /// `?dir=` — `asc`/`desc` for [`Self::sort`].
    pub dir: Signal<String>,
    /// The live cursor (GH #166), as one signal: `""` (no cursor),
    /// `after:<token>` or `before:<token>`. One signal makes the
    /// `after`+`before` pair Toasty rejects unrepresentable in the browser — no
    /// cross-write interleaving can produce it — and lets every result-set
    /// transition clear pagination with a single write. Written through
    /// `cursor_after` / `cursor_before` / `cursor_none`, read through
    /// `split_cursor`.
    pub cursor: Signal<String>,
    /// `?group_by=` — the active grouping (`""` = ungrouped, GH #157).
    /// Seeded from the page-load state and changed via navigation
    /// (`?group_by=` links); no live control writes it yet, so it persists
    /// across in-place reruns. A future control writing it must clear the
    /// cursor like the other result-set dimensions.
    pub group_by: Signal<String>,
    /// The bulk selection: comma-separated record keys, `""` when nothing is
    /// selected (GH #166). Row checkboxes carry no `checked` attribute —
    /// `bulk.js` sets `checked` from the transport after every swap and change
    /// — and the script writes the transport, whose bound `change` handler
    /// writes this signal, so a live rerun re-renders the boxes from the
    /// selection instead of dropping it. The shard carries the handle without
    /// reading it: the table needs it to bind the boxes, but a checkbox click
    /// must not reload rows.
    pub bulk: Signal<String>,
}

/// The live cursor wire format (GH #166): `after:<token>` / `before:<token>`,
/// with the empty string meaning "no cursor". One signal carries it, so the
/// browser can never hold both cursors at once.
const CURSOR_AFTER: &str = "after:";
const CURSOR_BEFORE: &str = "before:";

/// Wire value for a forward cursor (`?after=<token>`).
pub(crate) fn cursor_after(token: &str) -> String {
    format!("{CURSOR_AFTER}{token}")
}

/// Wire value for a backward cursor (`?before=<token>`).
pub(crate) fn cursor_before(token: &str) -> String {
    format!("{CURSOR_BEFORE}{token}")
}

/// Wire value for no cursor — what every result-set transition writes, and
/// what a fresh page seeds.
pub(crate) fn cursor_none() -> String {
    String::new()
}

/// Split the live cursor wire value into the `(after, before)` pair the loader
/// consumes. At most one side is ever `Some`: a value naming neither direction
/// (a tampered signal, or one the client never sent) degrades to "no cursor" —
/// the drop-pagination retry contract of GH #110 — rather than the pair error
/// of GH #155 that a URL carrying both cursors reaches at load time.
pub(crate) fn split_cursor(wire: &str) -> (Option<String>, Option<String>) {
    let wire = wire.trim();
    for (prefix, forward) in [(CURSOR_AFTER, true), (CURSOR_BEFORE, false)] {
        if let Some(token) = wire.strip_prefix(prefix) {
            let token = token.trim();
            if token.is_empty() {
                break;
            }
            return if forward {
                (Some(token.to_string()), None)
            } else {
                (None, Some(token.to_string()))
            };
        }
    }
    (None, None)
}

/// Whether `key` is selected in the live bulk wire (GH #166).
///
/// The wire is comma-delimited on both ends — `,a,b,`, empty when nothing is
/// selected — so the client-side `checked` binding tests membership with a
/// plain `contains(",<key>,")` instead of a substring test that would confuse
/// `b` with `ab`. [`parse_bulk_ids`](crate::panel) already ignores the empty
/// segments the delimiters produce, so the same wire is the form transport.
#[cfg(test)]
pub(crate) fn bulk_wire_contains(wire: &str, key: &str) -> bool {
    wire.split(',').any(|segment| segment == key)
}

/// One executed page of rows for `Table::render`.
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
    /// The app-level field name of the column (matches `TextColumn::name`).
    pub column: String,
    /// `true` for `dir=desc`.
    pub descending: bool,
}

/// Request-scoped table state, parsed from the current URL query.
///
/// The single parse point shared by loaders (the search term, ordering via
/// `Table::order_bys_for`) and render (active sort, toolbar values,
/// pagination links), so the URL is the one truth for list state. The fixed parameter
/// names assume one table per page — per-table prefixes are deferred until a
/// real page needs two tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableState {
    /// `?q=` — trimmed and clamped to `MAX_QUERY_TERM` chars; `None` when
    /// absent or blank.
    pub search: Option<String>,
    /// `?sort=` + `?dir=` — `None` when absent or blank.
    pub sort: Option<Sort>,
    /// `?after=` — encoded forward cursor.
    pub after: Option<String>,
    /// `?before=` — encoded backward cursor.
    pub before: Option<String>,
    /// `?filters=` — `key:value,key2:value2` (comma-separated, colon-delimited),
    /// bounded at parse time by `MAX_FILTERS_PARAM`/`MAX_FILTER_SEGMENTS`
    /// (GH #205).
    pub filters: HashMap<String, String>,
    /// `?filters=` segments that carry no `key:value` pair (GH #148): kept so
    /// `Table::unapplied_filters` can flag them (list banner, export 400)
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

/// Longest `?filters=` transport parsed (GH #205): the live shard hands this
/// the client-owned `filters` signal, and the router buffers shard bodies up
/// to megabytes — so the same bounded-echoed-state posture as
/// [`MAX_QUERY_TERM`](crate::query_term::MAX_QUERY_TERM) has to hold here,
/// where the transport is parsed, rather than at the shard that happens to read
/// it.
pub(crate) const MAX_FILTERS_PARAM: usize = 1024;

/// Most segments one `?filters=` transport may carry (GH #205): the byte cap
/// alone still admits a thousand one-character segments, and each surviving
/// segment becomes a map entry every rebuilt URL echoes.
pub(crate) const MAX_FILTER_SEGMENTS: usize = 32;

/// The one segment an over-long or over-full `?filters=` collapses to
/// (GH #205).
///
/// It carries no `key:value` pair, so it rides the GH #148 malformed channel:
/// [`Table::unapplied_filters`](crate::resource::Table::unapplied_filters)
/// flags it (the list warns, the export refuses with 400 instead of exporting
/// an over-broad CSV) and [`TableState::filters_param`] re-emits it, so the
/// warning survives pagination and sort links.
pub(crate) const FILTERS_OVERFLOW_SEGMENT: &str = "filters=overflow";

impl TableState {
    /// Parse the state from the request in `cx`.
    ///
    /// A blank or unknown query parses as neutral state rather than failing
    /// the request. Duplicate keys (`?filters=a&filters=b`) resolve to the
    /// first occurrence, so a repeated filter never vanishes: rejecting the
    /// duplicate would fail the whole decode, and answering empty state instead
    /// would drop every filter — including export's fail-closed guard (GH #93).
    /// Cursor errors still surface later, at decode time, where they are
    /// precise — including the conflicting `after` + `before` pair, which fails
    /// at load time (GH #155). Renders without a request context (e.g. unit
    /// tests) get neutral state.
    pub fn from_cx(cx: &Cx) -> Self {
        let Some(parts) = topcoat::context::try_request_context::<http::request::Parts>(cx) else {
            return Self::default();
        };
        let params = first_wins_query_params(parts.uri.query().unwrap_or(""));
        Self::from_parts(|key| params.get(key).map(String::as_str))
    }

    /// One shared constructor behind [`Self::from_cx`] and
    /// [`Self::from_live_args`] (GH #133, GH #206): every query-state parse
    /// funnels through one contract, so the public live-args entry point — the
    /// documented seam for a page owning its own signals — cannot be the
    /// looser one. `q` is trimmed and clamped to
    /// [`MAX_QUERY_TERM`](crate::query_term::MAX_QUERY_TERM), `dir` is
    /// trimmed before comparing, and the `filters` transport is bounded at
    /// [`MAX_FILTERS_PARAM`]/[`MAX_FILTER_SEGMENTS`].
    ///
    /// The live args arrive named at the single `match` below, where a
    /// transposed positional pair would not compile silently.
    fn from_parts<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Self {
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
                descending: get("dir").map(str::trim) == Some("desc"),
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
    ///
    /// This is the expensive half of a URL projection (a `format!` per pair,
    /// a sort, a join, and a percent-encode per byte), so a table render must
    /// encode it a bounded number of times — never once per row (GH #205).
    /// `TableState::row_url_base` exists to make that structural.
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
    /// [`Self::row_url_base`]'s dialog intent.
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

    /// The shared parameters of every row-action URL on one page, encoded
    /// once (GH #205).
    ///
    /// A row's action URL is this base plus the row's record key, so a table
    /// render pays for the filter transport — the expensive half of the
    /// projection — once, however many rows the page holds. Build it before
    /// the row loop and call [`RowUrlBase::delete_dialog`] per row; that pair
    /// is the full-state-plus-`delete` projection (GH #151), which keeps the
    /// cursors and never emits `open`.
    pub(crate) fn row_url_base(&self, path: &str) -> RowUrlBase {
        RowUrlBase(self.list_url(path))
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
    /// Shard requests hit the `table_search` shard's own endpoint, so
    /// [`Self::from_cx`] would see the endpoint URI — not the list page's
    /// query. The page hands over the current values of its interaction
    /// signals instead: the search term, the filter transport, the sort column
    /// and its direction, and the page's grouping. Live search resets
    /// pagination (`after`/`before` are always `None` — a new search is a new
    /// result set, same as the GET toolbar) and keeps the page's `group_by`.
    ///
    /// Every argument is client-owned by the time the shard reads it back, so
    /// this applies [`Self::from_cx`]'s bounds through the shared
    /// `Self::from_parts` (GH #206) — the public constructor is not the
    /// looser one.
    ///
    /// [`TableSignals::to_state`] is the shard's call site for this (GH #224):
    /// it reads the signals and passes their values here, so the shard never
    /// rebuilds state by hand.
    pub fn from_live_args(
        q: &str,
        filters_param: &str,
        sort: &str,
        dir: &str,
        group_by: &str,
    ) -> Self {
        // `after`/`before`/`delete`/`open` stay `None`: live search resets
        // pagination and the delete dialog with every keystroke (a new search
        // is a new result set, same as the GET toolbar), and the panel
        // renders the dialog outside the shard region for live tables
        // (GH #151). Missing keys read as absent through `from_parts`.
        Self::from_parts(|key| match key {
            "q" => Some(q),
            "filters" => Some(filters_param),
            "sort" => Some(sort),
            "dir" => Some(dir),
            "group_by" => Some(group_by),
            _ => None,
        })
    }

    /// Seed the page's interaction signals from this state (GH #224).
    ///
    /// The one state→signal conversion: the page owns the handles
    /// ([`TableSignals`]) and every control it renders writes them, so the
    /// values the page loaded with are what the signals start from. The
    /// cursor is one signal (GH #166) seeded from the `after`/`before` pair
    /// in this state; the bulk selection starts empty (the page never loads a
    /// selection); `group_by` seeds from the page-load value and persists
    /// across in-place reruns (GH #157).
    ///
    /// Call it with the *parsed* state, before normalizing: an unknown
    /// `?group_by=` seeds the signal as written, and the shard's
    /// [`TableSignals::to_state`] + `Table::normalize_state` drop it on the
    /// way back in, exactly as the GET path does (GH #153).
    ///
    /// Creates the signals, so it carries [`topcoat::runtime::signal`]'s
    /// contract: call it while a view is collecting signal declarations — the
    /// panel calls it from the live page's render, and the declarations ride
    /// that page's hoisted parts.
    pub fn to_signals(&self, cx: &Cx) -> TableSignals {
        TableSignals {
            q: signal(cx, || self.search.clone().unwrap_or_default()),
            filters: signal(cx, || self.filters_param().unwrap_or_default()),
            // The projection's own spelling of `?sort=`/`?dir=` (GH #153), so
            // the signal and every link agree on the direction word.
            sort: signal(cx, || {
                self.sort_pair()
                    .map(|(column, _)| column.to_string())
                    .unwrap_or_default()
            }),
            dir: signal(cx, || {
                self.sort_pair()
                    .map(|(_, dir)| dir.to_string())
                    .unwrap_or_else(|| "asc".to_string())
            }),
            cursor: signal(cx, || match (&self.after, &self.before) {
                (Some(token), _) => cursor_after(token),
                (None, Some(token)) => cursor_before(token),
                (None, None) => cursor_none(),
            }),
            group_by: signal(cx, || self.group_by.clone().unwrap_or_default()),
            bulk: signal(cx, String::new),
        }
    }
}

/// The live seam's other direction (GH #224): the signals the page owns,
/// read back into the request state the shard loads and renders with.
impl TableSignals {
    /// Rebuild request state from the live signals (GH #224).
    ///
    /// The one signal→state conversion: every value is client-owned by the time
    /// the shard reads it back, so it goes through
    /// [`TableState::from_live_args`] — the same `q` clamp and `filters` bound
    /// the GET path applies (GH #148, GH #205, GH #206) — and the one cursor
    /// wire is split into the `(after, before)` pair the loader consumes
    /// (GH #166). Pagination and the delete dialog always reset (GH #151); a
    /// token that does not decode fails loudly at load time (GH #158).
    pub fn to_state(&self) -> TableState {
        let mut state = TableState::from_live_args(
            &self.q.get(),
            &self.filters.get(),
            &self.sort.get(),
            &self.dir.get(),
            &self.group_by.get(),
        );
        (state.after, state.before) = split_cursor(&self.cursor.get());
        state
    }
}

/// One page's shared row-action URL parameters, encoded once (GH #205).
///
/// The base is [`TableState::list_url`] — every parameter a row's action URL
/// shares — so the filter transport is encoded once per render, not once per
/// row. Row-specific intents ([`Self::delete_dialog`]) append to it in the
/// projection's own order.
pub(crate) struct RowUrlBase(String);

impl RowUrlBase {
    /// The `?delete=<key>` confirmation-dialog opener for one row (GH #151).
    ///
    /// `self.0` is [`TableState::list_url`]'s output, which never carries
    /// `delete`, and `delete` is the projection's last parameter — so this is
    /// byte-for-byte what the one-pass projection builds, without re-encoding
    /// the parameters it shares with the rest of the page (GH #205).
    pub(crate) fn delete_dialog(&self, key: &str) -> String {
        let separator = if self.0.contains('?') { '&' } else { '?' };
        format!("{}{separator}delete={}", self.0, encode_query_value(key))
    }
}

// Action URL shapes (GH #206).
//
// `TableState` owns every table link's parameter vocabulary; these own the
// *path* shapes, so a route change has one edit site per shape instead of a
// hand-formatted `format!` at each render seam. The segment literals are shared
// with the panel's route table (`Panel::resource`), so the routes the panel
// registers and the links the table emits are spelled once.

/// The record placeholder the route table registers: `{id}`.
///
/// A route *pattern*, not a URL — the link helpers below take the encoded
/// record key instead.
pub(crate) const RECORD_ROUTE_PARAM: &str = "{id}";

/// Path segment of the list page's create page (GH #162).
pub(crate) const CREATE_ROUTE_SEGMENT: &str = "create";

/// Path segment of a row's edit page (GH #162).
pub(crate) const EDIT_ROUTE_SEGMENT: &str = "edit";

/// Path segment of a row's delete POST (GH #151).
pub(crate) const DELETE_ROUTE_SEGMENT: &str = "delete";

/// Path segment of the bulk-delete POST (GH #184).
pub(crate) const BULK_DELETE_ROUTE_SEGMENT: &str = "bulk-delete";

/// The row's `Edit` link: `{prefix}/{key}/edit` (GH #162).
pub(crate) fn row_edit_url(prefix: &str, key: &str) -> String {
    format!("{prefix}/{}/{EDIT_ROUTE_SEGMENT}", encode_path_segment(key))
}

/// The row's `View` link: `{prefix}/{key}` — the detail page (GH #187).
pub(crate) fn row_view_url(prefix: &str, key: &str) -> String {
    format!("{prefix}/{}", encode_path_segment(key))
}

/// The row delete form's POST target: `{prefix}/{key}/delete` (GH #151).
pub(crate) fn delete_action_url(prefix: &str, key: &str) -> String {
    format!(
        "{prefix}/{}/{DELETE_ROUTE_SEGMENT}",
        encode_path_segment(key)
    )
}

/// The list page's create link: `{list_path}/create` (GH #162).
pub(crate) fn create_page_url(list_path: &str) -> String {
    format!("{list_path}/{CREATE_ROUTE_SEGMENT}")
}

/// The bulk form's POST target: `{list_path}/bulk-delete` (GH #184).
pub(crate) fn bulk_delete_url(list_path: &str) -> String {
    format!("{list_path}/{BULK_DELETE_ROUTE_SEGMENT}")
}

/// Query-string pairs with the first occurrence winning.
///
/// A duplicate key keeps its first value rather than failing the decode:
/// rejecting it would fail the whole parse, and answering empty state instead
/// would silently drop filters and export's fail-closed guard (GH #93). Unknown
/// keys are ignored, like the typed decode.
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
/// An over-long or over-full transport is refused *whole* — never partially
/// applied, which would silently drop filters the caller did send — and the
/// refusal rides the GH #148 malformed channel as `FILTERS_OVERFLOW_SEGMENT`
/// (GH #205), so the list warns and the export 400s instead of running
/// unfiltered. The bound lives here, where the value is parsed: the live shard
/// hands this the client-owned `filters` signal, which the router buffers up to
/// megabytes of.
fn parse_filters_param(raw: &str) -> (HashMap<String, String>, Vec<String>) {
    // The length test first: it is O(1) and short-circuits the segment scan
    // for the oversized input this bound exists for.
    if raw.len() > MAX_FILTERS_PARAM || raw.split(',').count() > MAX_FILTER_SEGMENTS {
        return (HashMap::new(), vec![FILTERS_OVERFLOW_SEGMENT.to_string()]);
    }
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
pub(crate) fn encode_path_segment(value: &str) -> String {
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
pub(crate) fn row_dom_id(key: &str) -> String {
    dom_id("row", key)
}

/// Stable DOM id for a page-local group header (GH #219).
///
/// Same contract as [`row_dom_id`]: the header is injected, removed and moved
/// as the page is re-sorted, so the in-place morph needs an id derived from the
/// group label it belongs to rather than from its position in the page.
pub(crate) fn group_header_dom_id(label: &str) -> String {
    dom_id("group", label)
}

/// The one sanitizer behind both ids: `{prefix}-{token}-{hash}`.
fn dom_id(prefix: &str, key: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + key.len() + 14);
    out.push_str(prefix);
    out.push('-');
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use topcoat::context::{Cx, CxTestBuilder};

    use super::*;
    use crate::query_term::MAX_QUERY_TERM;

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

    #[test]
    fn live_and_url_constructors_share_one_contract() {
        // GH #206: `from_live_args` is the public seam a page owning its own
        // signals is documented to call, so it must apply the GET path's
        // bounds — `q` trimmed and clamped to `MAX_QUERY_TERM`, `dir` trimmed
        // before comparing — instead of being the looser of the two.
        let long = "x".repeat(MAX_QUERY_TERM + 10);
        let live = TableState::from_live_args(&long, "", "name", " desc ", "");
        let url = TableState::from_cx(&cx_with_query(&format!("q={long}&sort=name&dir=+desc+")));
        assert_eq!(live, url, "the two entry points must agree field for field");
        assert_eq!(
            live.search.as_deref().map(str::len),
            Some(MAX_QUERY_TERM),
            "the live constructor must clamp `q` like the GET path"
        );
        assert_eq!(
            live.sort,
            Some(Sort {
                column: "name".to_string(),
                descending: true,
            }),
            "both paths trim `dir` before comparing"
        );
        // Blank inputs agree on neutral state too.
        assert_eq!(
            TableState::from_live_args("", "", "", "", ""),
            TableState::default()
        );
        assert_eq!(
            TableState::from_cx(&cx_with_query("")),
            TableState::default()
        );
    }

    #[test]
    fn oversized_filters_transport_is_refused_whole_and_flagged() {
        // GH #205: the live shard hands `parse_filters_param` a client-owned
        // signal the router will buffer megabytes of. The transport is bounded
        // where it is parsed, and an oversized one is refused *whole* — never
        // partially applied — through the GH #148 malformed channel, so the
        // list warns and the export 400s instead of running unfiltered.
        let huge = format!("status:published,{}", "k:v,".repeat(2 * 1024 * 1024));
        assert!(huge.len() > MAX_FILTERS_PARAM);
        let state = TableState::from_live_args("", &huge, "", "", "");
        assert!(
            state.filters.is_empty(),
            "an oversized transport must not be partially applied"
        );
        assert_eq!(
            state.malformed_filters,
            vec![FILTERS_OVERFLOW_SEGMENT.to_string()]
        );
        // The refusal is bounded and survives into every rebuilt link.
        let param = state.filters_param().expect("the refusal must project");
        assert_eq!(param, FILTERS_OVERFLOW_SEGMENT);
        assert!(
            param.len() < MAX_FILTERS_PARAM,
            "the projected transport stays bounded, got {} bytes",
            param.len()
        );

        // A segment flood under the byte cap is refused the same way: 32
        // one-character pairs are small but would each cost a map entry.
        let flood = vec!["k:v"; MAX_FILTER_SEGMENTS + 1].join(",");
        assert!(flood.len() <= MAX_FILTERS_PARAM);
        let state = TableState::from_cx(&cx_with_query(&format!("filters={flood}")));
        assert!(state.filters.is_empty());
        assert_eq!(
            state.malformed_filters,
            vec![FILTERS_OVERFLOW_SEGMENT.to_string()]
        );

        // A transport inside both bounds still applies, unchanged.
        let ok = vec!["k:v"; MAX_FILTER_SEGMENTS].join(",");
        let state = TableState::from_live_args("", &ok, "", "", "");
        assert!(state.malformed_filters.is_empty());
        assert_eq!(state.filters.get("k").map(String::as_str), Some("v"));
    }

    #[test]
    fn live_args_filters_round_trip_through_transport() {
        // GH #136 §5: the live `filters` string is the same transport the URL
        // parses — encode/decode round-trips without loss.
        let state =
            TableState::from_live_args("Ada", "status:published,featured:true", "name", "desc", "");
        let param = state.filters_param().expect("live filters serialize");
        let back = TableState::from_live_args("Ada", &param, "name", "desc", "");
        assert_eq!(back.filters, state.filters);
        assert_eq!(back.search, state.search);
        assert_eq!(back.sort, state.sort);
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
        // GH #93: a duplicate param keeps its first value and never fails
        // open — answering empty state would drop every filter (and export's
        // fail-closed guard along with it).
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
    fn table_state_parses_filters_param() {
        // GH #136: relocated from the showcase
        // (`table_state_parses_filters_and_filter_expr`) — core owns the
        // parse contract, the showcase owns HTTP wiring.
        let cx = cx_with_query("filters=status:published,featured:true");
        let state = TableState::from_cx(&cx);
        assert_eq!(
            state.filters.get("status").map(String::as_str),
            Some("published")
        );
        assert_eq!(
            state.filters.get("featured").map(String::as_str),
            Some("true")
        );
        assert!(state.malformed_filters.is_empty());
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

    #[test]
    fn client_transport_parses_to_the_client_value() {
        // The literals are the ones `filters.js` composes: the same fixtures
        // run in `crates/argentum-ui/assets/filters.test.js`. Pinning them
        // here joins the two halves — change `encode_filter_component` and the
        // browser's literal stops decoding to its value, change the browser's
        // encoder and the literal it emits stops matching this test.
        let cases = [
            ("name:Smith%2C John", "name", "Smith, John"),
            ("name:a%3Ab", "name", "a:b"),
            ("name:100%25", "name", "100%"),
            ("name:Ada Lovelace", "name", "Ada Lovelace"),
            ("name:%253A%252C%2525", "name", "%3A%2C%25"),
            // The key is escaped with the value.
            ("a%2Cb%3Ac:x", "a,b:c", "x"),
        ];
        for (transport, key, value) in cases {
            let (filters, malformed) = parse_filters_param(transport);
            assert!(
                malformed.is_empty(),
                "{transport:?} must parse clean, got {malformed:?}"
            );
            assert_eq!(
                filters.get(key).map(String::as_str),
                Some(value),
                "{transport:?} must decode to {value:?}"
            );
        }
        // The empty value clears the filter: `filters.js` emits no segment.
        let (empty, malformed) = parse_filters_param("");
        assert!(empty.is_empty() && malformed.is_empty());
        // Several controls join with `,`, and a value's own comma stays inside
        // its segment.
        let (pair, malformed) = parse_filters_param("status:published,q:a%2Cb");
        assert!(malformed.is_empty(), "got {malformed:?}");
        assert_eq!(pair.get("status").map(String::as_str), Some("published"));
        assert_eq!(pair.get("q").map(String::as_str), Some("a,b"));
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

    #[test]
    fn group_header_dom_ids_are_stable_and_distinct_from_row_ids() {
        // GH #219: the injected group header is moved and removed as the page
        // is re-sorted, so it needs a stable id of its own — and it must never
        // collide with a row id, or the morph would follow the wrong element.
        assert_eq!(
            group_header_dom_id("draft"),
            group_header_dom_id("draft"),
            "ids must be stable per label"
        );
        let draft = group_header_dom_id("draft");
        assert!(
            draft.starts_with("group-draft-"),
            "the id names its group, got {draft:?}"
        );
        assert!(
            group_header_dom_id("New York").starts_with("group-New-York-"),
            "labels sanitize to HTML-safe tokens, got {:?}",
            group_header_dom_id("New York")
        );
        // Two labels sanitizing to one token must not collide, and a group id
        // is never a row id even for the same text.
        assert_ne!(
            group_header_dom_id("New York"),
            group_header_dom_id("New-York")
        );
        assert_ne!(group_header_dom_id("draft"), row_dom_id("draft"));
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
    fn projection_row_url_base_adds_the_delete_dialog_key() {
        // GH #153 (GH #151): full state including cursors + `delete=key`;
        // never `open`.
        let source = populated_state();
        let mut expected = source.clone();
        expected.delete = Some("row-9".to_string());
        expected.open = None;
        // Spelled as the page's shared base plus the row key — exactly what a
        // table render does per row (GH #205).
        assert_eq!(
            reparse(&source.row_url_base("/admin/users").delete_dialog("row-9")),
            expected
        );
        // A state with nothing else to project still opens the dialog.
        assert_eq!(
            TableState::default()
                .row_url_base("/admin/users")
                .delete_dialog("row-9"),
            "/admin/users?delete=row-9"
        );
    }
    /// GH #166: the live cursor travels as one wire value, so the browser can
    /// never hold `after` and `before` at once — the pair Toasty rejects
    /// (GH #155) is unreachable from the live path.
    #[test]
    fn cursor_wire_carries_at_most_one_direction() {
        assert_eq!(cursor_after("tok"), "after:tok");
        assert_eq!(cursor_before("tok"), "before:tok");
        assert_eq!(cursor_none(), "");
        assert_eq!(
            split_cursor(&cursor_after("tok")),
            (Some("tok".into()), None)
        );
        assert_eq!(
            split_cursor(&cursor_before("tok")),
            (None, Some("tok".into()))
        );
        assert_eq!(split_cursor(&cursor_none()), (None, None));
        // Whitespace from the signal is tolerated, like the GET path's trims.
        assert_eq!(
            split_cursor("  after: tok  "),
            (Some("tok".to_string()), None)
        );
        // A tampered or half-written value degrades to no cursor (GH #110's
        // drop-pagination retry contract) instead of erroring the table.
        assert_eq!(split_cursor("tok"), (None, None));
        assert_eq!(split_cursor("after:"), (None, None));
        assert_eq!(split_cursor("before:"), (None, None));
        // Only the prefix is a direction; a token may contain colons.
        assert_eq!(split_cursor("after:a:b"), (Some("a:b".to_string()), None));
    }

    /// GH #166: the bulk wire is delimited on both ends so membership is exact
    /// (`b` is not selected by `,ab,`), and the delimiters ride through the
    /// form transport the bulk handler already parses.
    #[test]
    fn bulk_wire_membership_is_exact() {
        let wire = ",row-1,row-2,";
        assert!(bulk_wire_contains(wire, "row-1"));
        assert!(bulk_wire_contains(wire, "row-2"));
        assert!(!bulk_wire_contains(wire, "row"));
        assert!(!bulk_wire_contains(wire, "row-1a"));
        assert!(!bulk_wire_contains(",row-12,", "row-1"));
        assert!(!bulk_wire_contains("", "row-1"));
    }
}
