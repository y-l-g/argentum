//! [`Table`] HTML rendering: `render`/`render_with_state`/`render_skeleton` plus the chrome.
//!
//! Grouping is page-local and interleaved and a page encodes the
//! filter transport once.

use std::borrow::Cow;

use tablo_ui::{
    ButtonSize, ButtonVariant, alert_dialog, button, button_variants, dialog_content,
    dialog_description, dialog_footer, dialog_header, dialog_title, icons, input as ui_input,
    pagination, pagination_content, pagination_item, pagination_next, pagination_previous, table,
    table_body, table_cell, table_head, table_header, table_row,
};
use topcoat::{Result, context::Cx, icon::icon, runtime::Event, view::*};

use super::{
    super::{
        ColumnWidth,
        column::NARROW_DEFAULT_PERCENT,
        filter::Filter,
        state::{
            TablePage, TableSignals, TableState, bulk_delete_url, delete_action_url,
            group_header_dom_id, row_dom_id, row_edit_url, row_view_url,
        },
    },
    GroupKey, NormalizedState, RowActions, RowKey, Table,
};

/// Keystroke-quiet delay before a live search input reloads the table
/// (~150-250ms): `assets/live-search.js` waits this long after the
/// last keystroke, then forwards the value through the bound transport below,
/// so typing "published" triggers one reload instead of nine. The forwarded
/// write is an ordinary signal write, so Topcoat's abort-in-flight
/// coalescing still applies to the resulting rerun.
pub(crate) const LIVE_SEARCH_DEBOUNCE_MS: u32 = 200;

/// The accessible reason a bulk checkbox disabled by the per-row policy carries
/// the row's `Delete` is denied, so selecting it could only produce
/// a batch the handler refuses. Read aloud by a screen reader in place of the
/// checkbox's usual "Select row" label, and offered as the pointer tooltip too.
///
/// A row with no allowed action at all carries the same reason on the badge
/// that stands in for its links ([`LOCKED_ROW_LABEL`]).
pub(crate) const DENIED_ROW_REASON: &str = "You cannot delete this row";

/// The label a row whose policy allows no action renders in place of its
/// row-action links: the cell is never left empty, so a locked row reads as a
/// state rather than as missing chrome. Generic on purpose — the framework
/// knows only the [`RowActions`] outcome, not whether the record is
/// SSO-managed or a removed placeholder — with [`DENIED_ROW_REASON`] as its
/// tooltip, the same reason the row's disabled bulk checkbox carries.
pub(crate) const LOCKED_ROW_LABEL: &str = "Locked";

/// The readability floor one [`ColumnWidth::Wide`](super::super::ColumnWidth::Wide)
/// column contributes to the table's `min-width`, in whole rem.
///
/// A wide column declares no width — it takes what the declared columns leave —
/// so a sum of declared widths alone would let it crush to zero on a narrow
/// viewport (the measured 38px cells at 480px). Six rem keeps body text
/// readable and, summed across the wide columns, trips the wrapper's
/// horizontal scroll before the fixed layout crushes them.
const WIDE_COLUMN_MIN_REM: u8 = 6;

/// The share of the table the bulk-selection column claims: one
/// checkbox plus the cell's `p-3` padding at the widths a list is read at. A
/// percentage, not a length: the column keeps its share as the table narrows,
/// and the columns that declare none keep theirs.
const BULK_COLUMN_PERCENT: u8 = 5;

/// The most of the table the kind defaults claim together.
///
/// The defaults are shares of the table, and the columns that declare none
/// take what they leave: a total over 100% gives those columns no space at
/// all, and `table-fixed` renders a column with no space at zero width, header
/// text included. The budget keeps the rest of the table for them whatever the
/// column set.
const DEFAULT_WIDTH_BUDGET_PERCENT: u8 = 60;

/// The share a kind default claims, scaled down when the table's defaults
/// together exceed [`DEFAULT_WIDTH_BUDGET_PERCENT`].
fn scaled_default_percent(nominal: u8, total: u32) -> u8 {
    if total <= u32::from(DEFAULT_WIDTH_BUDGET_PERCENT) {
        return nominal;
    }
    let scaled = u32::from(nominal) * u32::from(DEFAULT_WIDTH_BUDGET_PERCENT) / total;
    // `scaled` is at most the budget, so the conversion cannot fail.
    u8::try_from(scaled).unwrap_or(DEFAULT_WIDTH_BUDGET_PERCENT)
}

/// The `style` value a kind default emits.
fn default_width_style(percent: u8) -> Cow<'static, str> {
    Cow::Owned(format!("width: {percent}%"))
}

/// One filter control: a labelled `<select data-filter-name=…>` carrying the
/// `value`/`label` pairs, with the leading empty "All" option that clears the
/// filter.
///
/// The empty value is reserved for that clear-filter option. Every pair in
/// `options` renders verbatim, so a caller whose declared options can include
/// `""` supplies the label that option shows: [`Filter::Select`] passes
/// `"All"`, the label the empty value already carries, while
/// [`Filter::Variant`] passes the key itself.
///
/// The control has no `name`, so it never submits on its own; `filters.js`
/// composes it into the hidden `filters` transport.
fn filter_select<'a>(
    cx: &'a Cx,
    label: &str,
    name: &str,
    options: Vec<(String, String)>,
    current: &str,
) -> BoxView<'a> {
    let label = label.to_string();
    let name = name.to_string();
    let aria = label.clone();
    let current = current.to_string();
    view! {
        cx =>
        <label class="flex items-center gap-2 text-sm text-muted-foreground">
            (label)
            <select
                data-filter-name=(name)
                aria-label=(aria)
                class="flex h-9 rounded-md border border-border bg-background px-3 py-1 text-sm shadow-xs"
            >
                <option value="" selected=(current.is_empty())>"All"</option>
                for (value, text) in options {
                    <option value=(value.clone()) selected=(current == value)>
                        (text)
                    </option>
                }
            </select>
        </label>
    }
    .boxed()
}

/// The hidden inputs a table toolbar carries across its submit, in the order
/// given; an input whose state holds no value renders nothing.
fn hidden_state_inputs<'a>(cx: &'a Cx, inputs: Vec<(&'static str, Option<String>)>) -> BoxView<'a> {
    let fields: Vec<BoxView<'a>> = inputs
        .into_iter()
        .filter_map(|(name, value)| {
            value.map(|value| {
                view! { cx => <input type="hidden" name=(name) value=(value)> }.boxed()
            })
        })
        .collect();
    view! {
        cx =>
        for field in fields {
            (field)
        }
    }
    .boxed()
}

/// The width every column of one render declares: one `style` value
/// per declared column, in column order, plus the two chrome columns.
/// `None` is a column that declares no width — a wide column, which takes a
/// share of what the declared ones leave.
///
/// `actions_min` is the actions column's content floor for its body cells
/// (the header carries the share *and* the floor); `table_min_width` is the
/// table-level floor — the sum of the declared widths — that lets the
/// wrapper's `overflow-x-auto` scroll on a narrow viewport instead of
/// crushing the cells.
struct ColumnWidths {
    cells: Vec<Option<Cow<'static, str>>>,
    bulk: Option<Cow<'static, str>>,
    actions: Option<Cow<'static, str>>,
    actions_min: Option<Cow<'static, str>>,
    table_min_width: Option<Cow<'static, str>>,
}

impl<M> Table<M> {
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
    /// Composes the synced `tablo-ui` primitives and Token classes
    /// (`border-border` on the chrome, `bg-background`/`shadow-xs` on the
    /// toolbar controls, `text-muted-foreground` on the captions) — no raw
    /// colors, no `ac-*`.
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
    /// `cx` — the seam a live-search shard needs: shard requests hit
    /// the `table_search` shard's own endpoint, so `TableState::from_cx` would
    /// see the endpoint URI, not the list page's `?q=/filters/sort`. Callers
    /// pass the page's state (or shard args rebuilt via
    /// [`TableSignals::to_state`]) and the list URL explicitly.
    ///
    /// Normalizes the state it is handed, so a page calling this
    /// directly needs no knowledge of `NormalizedState`; a caller that
    /// already normalized once per request goes through
    /// `Self::render_normalized` instead.
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
        self.render_normalized(cx, page, &self.normalize_state(state), path)
            .await
    }

    /// [`Self::render_with_state`] with the state already normalized
    /// the request entry normalizes once and every seam below takes
    /// the proof, so a live list request never normalizes the same state
    /// twice.
    pub(crate) async fn render_normalized<'a>(
        &self,
        cx: &'a Cx,
        page: TablePage<M>,
        state: &NormalizedState,
        path: &str,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        self.render_inner(cx, page, state, path, None).await
    }

    /// Render the interactive body for a live table: the same
    /// presentation as [`Self::render_with_state`], with the sort links, the
    /// pager, the filter transport, and the empty-state clear links bound to
    /// `signals` — each interaction writes a signal and the browser morphs the
    /// shard's new output in place, without a navigation or a scroll jump.
    /// Every bound control keeps its real `href`/form, so a page without JS
    /// still navigates as before.
    ///
    /// The row-delete dialog is not part of this output: it lives
    /// outside the region a rerun swaps, rendered once by the page that owns
    /// the signals, so a caller rendering only through this method renders
    /// [`Self::render_delete_dialog`] itself to keep the `?delete=` fallback.
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
        self.render_live_normalized(cx, page, &self.normalize_state(state), path, signals)
            .await
    }

    /// [`Self::render_live_with_state`] with the state already normalized
    /// the `table_search` shard normalizes once and renders
    /// through here.
    pub(crate) async fn render_live_normalized<'a>(
        &self,
        cx: &'a Cx,
        page: TablePage<M>,
        state: &NormalizedState,
        path: &str,
        signals: TableSignals,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model + Send + Sync + 'static,
    {
        self.render_inner(cx, page, state, path, Some(signals))
            .await
    }

    async fn render_inner<'a>(
        &self,
        cx: &'a Cx,
        page: TablePage<M>,
        state: &NormalizedState,
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
        let delete_prefix = self.delete_prefix.clone();
        let with_actions = self.with_actions();
        let with_bulk = self.bulk_enabled();
        // Record keys feed URLs and bulk values, which handlers resolve as
        // the typed PK: chrome without `pk` would emit display keys
        // the handlers 404 on, so fail loud like a missing row key.
        if (with_actions || with_bulk) && self.record_key.is_none() {
            return Err(std::io::Error::other(
                "Table::render: action chrome needs a record key — declare one via Table::pk(|row| ..)",
            )
            .into());
        }
        let record_key = self.record_key.clone();
        let head = self
            .render_thead(cx, state, path, with_actions, with_bulk, signals.as_ref())
            .await?;
        let show_search = self.search_enabled();
        let search_bar = if show_search {
            Some(self.render_search_bar(cx, state, path).await?)
        } else {
            None
        };
        let show_filters = self.filter_bar_enabled();
        let filter_bar = if show_filters {
            Some(
                self.render_filter_bar(cx, state, path, signals.as_ref())
                    .await?,
            )
        } else {
            None
        };
        let bulk_bar_view = self.render_bulk_bar(cx, signals.as_ref());
        let pager = self
            .render_pager(cx, state, path, &page, signals.as_ref())
            .await?;
        let filter_warning = self.render_filter_warning(cx, state, path);
        // The declared grouping, when `?group_by=` names it. Read
        // before the row projection so each row can carry its group label,
        // which the page-local shim orders by.
        let group_key = self.effective_group_key(state);
        let row_data = self.row_views(
            state,
            path,
            &page,
            &row_key,
            record_key.as_ref(),
            group_key.as_ref(),
        );
        // The confirmation dialog lives with the delete chrome and
        // ships closed, so a row control opens it in place. A live
        // output carries none: the page that owns the signals renders it once,
        // outside the region a rerun swaps (`panel::resource_list_live`). The
        // dialog renders on every page with delete chrome, not only under
        // `?delete=`, so without this gate every shard rerun would morph a
        // second copy — and its ids — into the page.
        let delete_dialog = if signals.is_none() {
            self.render_delete_dialog_normalized(cx, state).await?
        } else {
            None
        };

        // Body-only branch: the empty and rows pages share the one
        // chrome wrapper built below — only the table body differs. Group
        // headers and the pager exist solely on rows pages: an empty page
        // renders the honest empty cell instead (its pager would be empty
        // anyway, and grouping an empty page yields no headers).
        //
        // The declared widths are a property of the columns, not of the row,
        // so they are resolved once here: the same CSS for every row, and a
        // `for` whose expression names `self.columns` would carry the table's
        // borrow into the view.
        let ColumnWidths {
            cells: cell_widths,
            actions_min,
            table_min_width,
            ..
        } = self.column_widths();
        let mut pager_views: Vec<BoxView<'_>> = Vec::new();
        let body: BoxView<'_> = if page.rows.is_empty() {
            let empty_cell = self
                .render_empty_cell(cx, state, path, with_actions, with_bulk, signals.as_ref())
                .await?;
            view! {
                cx =>
                table(
                    attrs: attributes! { class="table-fixed" style=(table_min_width.as_deref()) },
                    (head)
                    (empty_cell)
                )
            }
            .boxed()
        } else {
            pager_views = pager;
            // One table body for grouped and ungrouped pages: each grouped
            // row carries the header its group's first row owns, so
            // the header lands inside the table immediately above its own
            // rows instead of a count legend stacked over an ungrouped table.
            let header_colspan =
                self.columns.len() + usize::from(with_bulk) + usize::from(with_actions);
            // The one dialog every row control on this table opens;
            // empty without delete chrome, where no control renders one.
            let delete_dialog_id = delete_prefix
                .as_deref()
                .map(Self::delete_dialog_dom_id)
                .unwrap_or_default();
            view! {
                cx =>
                // The table-level layout is a static class: Tailwind sees the
                // literal, and the decision carries no per-column value. Each
                // column's width, which does, rides the `th`/`td` inline
                // `style`. Fixed layout is what stops a filter or a
                // page change from re-measuring the columns. The `min-width`
                // is the sum of those declared widths: with `w-full` the
                // table never exceeds its container on its own, so without
                // the floor the wrapper's `overflow-x-auto` never scrolls.
                table(
                    attrs: attributes! { class="table-fixed" style=(table_min_width.as_deref()) },
                    (head)
                    table_body(
                        #[key(row.key.as_str())]
                        for row in &row_data {
                            let key_for_row = row.key.clone();
                            let key_for_select = row.record_id.clone();
                            let view_for_row = row.view_url.clone();
                            let edit_for_row = row.edit_url.clone();
                            let open_for_row = row.delete_url.clone();
                            let delete_action_for_row = row.delete_action.clone();
                            let delete_dialog_for_row = delete_dialog_id.clone();
                            let selectable_for_row = row.selectable;
                            let locked_for_row = row.locked;
                            let row_dom_id = row_dom_id(&key_for_row);
                            if let Some(header) = row.group_header.clone() {
                                table_row(
                                    attrs: attributes! { id=(header.dom_id) },
                                    table_cell(
                                        attrs: attributes! {
                                            colspan=(header_colspan)
                                            class="px-4 py-2 bg-muted text-sm font-medium"
                                        },
                                        (header.text)
                                    )
                                )
                            }
                            table_row(
                                attrs: attributes! { id=(row_dom_id) },
                                if with_bulk {
                                    if selectable_for_row {
                                        table_cell(
                                            <input
                                                type="checkbox"
                                                value=(key_for_select)
                                                aria-label="Select row"
                                                data-row-select=""
                                            >
                                        )
                                    } else {
                                        table_cell(
                                            <input
                                                type="checkbox"
                                                value=(key_for_select)
                                                aria-label=(DENIED_ROW_REASON)
                                                title=(DENIED_ROW_REASON)
                                                data-row-select=""
                                                disabled=""
                                            >
                                        )
                                    }
                                }
                                // `row.cells` is built column-for-column, so the
                                // zip pairs each cell with the column that owns
                                // its width. The cell repeats the
                                // width its header declares and truncates:
                                // under the table's fixed layout a value wider
                                // than the column clips to an ellipsis instead
                                // of stretching the column. `truncate` is a
                                // static class, which Tailwind does generate —
                                // only the per-column width has to be data.
                                for (cell, width) in row.cells.iter().zip(&cell_widths) {
                                    table_cell(
                                        attrs: attributes! { class="truncate" style=(width.as_deref()) },
                                        (cell.clone())
                                    )
                                }
                                // Every row carries the actions cell its header
                                // declares, even a row locked out of every
                                // link. The cell repeats only the column's
                                // content floor — never a width share, which
                                // the header row owns — so the buttons fit
                                // instead of spilling past the table.
                                if with_actions {
                                    table_cell(
                                        attrs: attributes! { style=(actions_min.as_deref()) },
                                        if locked_for_row {
                                            <span
                                                class="inline-flex items-center rounded-md border border-border bg-muted px-2 py-1 text-xs font-medium text-muted-foreground"
                                                title=(DENIED_ROW_REASON)
                                            >
                                                (LOCKED_ROW_LABEL)
                                            </span>
                                        }
                                        if !locked_for_row {
                                            <div class="flex gap-2">
                                                if let Some(url) = view_for_row {
                                                    <a
                                                        href=(url)
                                                        class=(button_variants(
                                                            ButtonVariant::Outline,
                                                            ButtonSize::Md,
                                                        ))
                                                    >
                                                        "View"
                                                    </a>
                                                }
                                                if let Some(url) = edit_for_row {
                                                    <a
                                                        href=(url)
                                                        class=(button_variants(
                                                            ButtonVariant::Outline,
                                                            ButtonSize::Md,
                                                        ))
                                                    >
                                                        "Edit"
                                                    </a>
                                                }
                                                if let (Some(url), Some(action)) = (
                                                    open_for_row,
                                                    delete_action_for_row,
                                                ) {
                                                    <a
                                                        href=(url)
                                                        data-row-delete-trigger=(delete_dialog_for_row)
                                                        data-row-delete-action=(action)
                                                        class=(button_variants(
                                                            ButtonVariant::Destructive,
                                                            ButtonSize::Md,
                                                        ))
                                                    >
                                                        "Delete"
                                                    </a>
                                                }
                                            </div>
                                        }
                                    )
                                }
                            )
                        }
                    )
                )
            }
            .boxed()
        };

        // The refresh control: a live table's region re-renders when
        // a signal its shard tracked changes, and a mutation changes rows the
        // tracked inputs do not describe — the query is the same, the data is
        // not. One write that means "re-read the table" is therefore the
        // mutation's only honest in-place effect: the client bumps this
        // revision token and the shard re-runs the query, morphing and
        // re-hydrating the region through the seam that already exists.
        //
        // The signal is declared here, inside the shard's own output, so it
        // belongs to the shard's content scope: its id derives from the shard
        // invocation's identity and this call site, so it is stable across
        // reruns and distinct per shard, and the runtime keeps its value when
        // the declaration renders again. Reading it for the input's initial
        // value is what declares the dependency; the token itself is opaque.
        //
        // A static table renders no control at all: it has no shard to re-run,
        // its region is inert markup, and the client replaces it wholesale
        // So the control's presence *is* the page's answer to
        // "can this table refresh in place?".
        let revision_attrs = signals.as_ref().map(|_| {
            let revision = topcoat::runtime::signal(cx, || "0".to_string());
            attributes! {
                cx =>
                type="hidden"
                value=(revision.get())
                :value=$(revision.get())
                @change=$(|e: Event| revision.set(e.target.value))
                data-table-revision=""
            }
        });

        // One chrome wrapper for both branches: search bar, filter bar, bulk
        // bar, warning, table body, pager, dialog, inside the
        // `data-boundary` region the morph swaps.
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
                if let Some(attrs) = revision_attrs {
                    <input (attrs)>
                }
                if let Some(warning) = filter_warning {
                    (warning)
                }
                (body)
                for p in pager_views {
                    (p)
                }
                if let Some(dialog) = delete_dialog {
                    (dialog)
                }
            </div>
        };
        Ok(view! { cx => <div data-boundary="table">(inner)</div> }.boxed())
    }

    /// The bulk-delete bar and its confirmation dialog, or the
    /// placeholder that keeps the chrome's node order stable when
    /// [`Self::bulk_enabled`] is off.
    ///
    /// One destructive form for the whole page: the transport is fed by the
    /// row checkboxes (`bulk.js`) and ships `,a,b,`-delimited, while on a live
    /// table the selection lives in a signal instead, so a shard
    /// rerun re-renders the transport from the selection rather than dropping
    /// it. The trigger ships enabled: the confirmation dialog gates
    /// the write and reads the selection when it opens, so an empty selection
    /// is answered by the dialog rather than by a disabled control whose state
    /// has to be kept in step across a live swap.
    ///
    /// The dialog lives *inside* the form so its `confirm` marker ships with
    /// the same payload as the selection — the confirm button is an ordinary
    /// submit of that form, and the handler refuses a POST without the marker.
    /// It renders closed and opens client-side (`showModal`) rather than
    /// through a runtime signal: the trigger is `type="button"`, so opening
    /// the dialog is not a result-set change and must not reload the table.
    fn render_bulk_bar<'a>(&self, cx: &'a Cx, signals: Option<&TableSignals>) -> BoxView<'a> {
        if !self.bulk_enabled() {
            return view! { cx => <span></span> }.boxed();
        }
        let prefix = self
            .delete_prefix
            .clone()
            .expect("bulk chrome rides the delete prefix (see bulk_enabled)");
        let bulk_action = bulk_delete_url(&prefix);
        let csrf = crate::csrf::current_token(cx);
        // Stable ids so the dialog's confirm button can submit this form
        // from inside the dialog.
        let bulk_form_id = format!("{}-bulk-form", prefix.replace('/', "-"));
        let bulk_dialog_id = format!("{bulk_form_id}-confirm");
        let bulk_dialog_title_id = format!("{bulk_dialog_id}-title");
        let bulk_dialog_description_id = format!("{bulk_dialog_id}-description");
        // No visible `ids` field: the transport is fed by the row
        // checkboxes (`bulk.js`) and ships `,a,b,`-delimited. On a live
        // table the selection lives in a signal instead, so a
        // shard rerun re-renders the transport from the selection rather
        // than dropping it.
        let transport_attrs = match signals {
            Some(signals) => {
                let bulk = signals.bulk.clone();
                attributes! {
                    cx =>
                    type="hidden"
                    name="ids"
                    :value=$(bulk.get())
                    @change=$(|e: Event| bulk.set(e.target.value))
                }
            }
            None => attributes! { cx => type="hidden" name="ids" value="" },
        };
        view! {
            cx =>
            <form
                method="post"
                action=(bulk_action)
                class="flex gap-2 p-3 border-b border-border"
                data-bulk-form=""
                data-mutation-submit=""
                id=(bulk_form_id.clone())
            >
                (crate::csrf::field(cx, &csrf))
                <input (transport_attrs)>
                button(
                    variant: ButtonVariant::Destructive,
                    size: ButtonSize::Md,
                    attrs: attributes! { type="button" data-bulk-confirm-trigger="" },
                    "Bulk Delete"
                )
                // Destructive confirm: a batch is the one place a
                // misclick costs many rows, so it asks first — the same
                // alert-dialog pattern the row delete already uses.
                alert_dialog(
                    open: false,
                    attrs: attributes! {
                        id=(bulk_dialog_id.clone())
                        data-bulk-confirm-dialog=""
                        aria-labelledby=(bulk_dialog_title_id.clone())
                        aria-describedby=(bulk_dialog_description_id.clone())
                    },
                    dialog_content(
                        dialog_header(
                            dialog_title(
                                attrs: attributes! { id=(bulk_dialog_title_id.clone()) },
                                "Delete the selected records?"
                            )
                            dialog_description(
                                attrs: attributes! {
                                    id=(bulk_dialog_description_id.clone())
                                    data-bulk-confirm-description=""
                                },
                                "This action cannot be undone."
                            )
                        )
                        dialog_footer(
                            button(
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Md,
                                attrs: attributes! { type="button" data-dialog-close="" },
                                "Cancel"
                            )
                            <input type="hidden" name="confirm" value="1">
                            button(
                                variant: ButtonVariant::Destructive,
                                size: ButtonSize::Md,
                                attrs: attributes! { type="submit" },
                                "Delete"
                            )
                        )
                    )
                )
            </form>
        }
        .boxed()
    }

    /// Project the loaded page into the row presentation the template renders.
    ///
    /// Precomputed so template bodies capture only owned data — the lazy view
    /// outlives the render call, so it must never borrow `self` or `page`.
    ///
    /// The per-row delete URL opens the confirmation dialog on the list page
    /// (`?delete=<key>`); the per-row edit URL links to `{prefix}/{key}/edit`.
    /// Both — and the bulk checkbox values — carry the *record* key, resolved by
    /// handlers as the model's typed PK; the display `key` stays on keyed diffs
    /// and DOM ids.
    ///
    /// The chrome prefixes say which links the table *can* render; the
    /// [`Table::row_actions`] policy says which of them *this* record may use.
    /// A denied action emits no URL, and a row denied `delete` renders its bulk
    /// checkbox disabled. The policy is consulted only when a prefix is wired.
    ///
    /// The delete URL's shared parameters are encoded once for the whole page,
    /// because the filter transport is the expensive half and rebuilding it per
    /// row is work a client can inflate with one oversized `?filters=`.
    ///
    /// Page-local grouping is display-only: `group_by` is a bare key closure with
    /// no lens, so no `ORDER BY` is derivable and a group cannot span pages. The
    /// shim therefore reorders *this page's* rows by the group label — a stable
    /// sort, so rows keep the query's order inside their group — and hangs each
    /// group's header off its first row. The query, its cursors and the export
    /// keep the declared ordering.
    ///
    /// Row keys must be injective within a page: duplicates corrupt keyed diffs
    /// and bulk selection.
    fn row_views(
        &self,
        state: &NormalizedState,
        path: &str,
        page: &TablePage<M>,
        row_key: &RowKey<M>,
        record_key: Option<&RowKey<M>>,
        group_key: Option<&GroupKey<M>>,
    ) -> Vec<RowView>
    where
        M: toasty::schema::Model,
    {
        let delete_url_base = self
            .delete_prefix
            .as_ref()
            .map(|_| state.row_url_base(path));
        let gated = self.delete_prefix.is_some()
            || self.edit_prefix.is_some()
            || self.view_prefix.is_some();
        let mut row_data: Vec<RowView> = page
            .rows
            .iter()
            .map(|row| {
                let key = row_key(row);
                let record_id = record_key.map(|f| f(row)).unwrap_or_default();
                let actions = if gated {
                    self.actions_for(row)
                } else {
                    RowActions::ALL
                };
                let cells: Vec<String> = self
                    .columns
                    .iter()
                    .map(|col| col.render_cell(row))
                    .collect();
                let edit_url = self
                    .edit_prefix
                    .as_ref()
                    .filter(|_| actions.edit)
                    .map(|prefix| row_edit_url(prefix, &record_id));
                let view_url = self
                    .view_prefix
                    .as_ref()
                    .filter(|_| actions.view)
                    .map(|prefix| row_view_url(prefix, &record_id));
                let delete_url = delete_url_base
                    .as_ref()
                    .filter(|_| actions.delete)
                    .map(|base| base.delete_dialog(&record_id));
                // The shared dialog's POST target for this row: the
                // row control hands it over before opening the dialog, so the
                // action and the control come from the one policy decision.
                let delete_action = self
                    .delete_prefix
                    .as_ref()
                    .filter(|_| actions.delete)
                    .map(|prefix| delete_action_url(prefix, &record_id));
                // A row with no allowed action keeps its actions cell: the
                // template renders the locked badge in place of the links.
                // Read before the URLs move into the view below.
                let locked = view_url.is_none() && edit_url.is_none() && delete_url.is_none();
                RowView {
                    key,
                    record_id,
                    cells,
                    view_url,
                    edit_url,
                    delete_url,
                    delete_action,
                    selectable: actions.delete,
                    locked,
                    group: group_key.map(|group| group(row)),
                    group_header: None,
                }
            })
            .collect();
        if group_key.is_some() {
            row_data.sort_by(|a, b| a.group.cmp(&b.group));
            let mut start = 0;
            while start < row_data.len() {
                let label = row_data[start].group.clone().unwrap_or_default();
                let mut end = start;
                while end < row_data.len() && row_data[end].group.as_deref() == Some(label.as_str())
                {
                    end += 1;
                }
                // The count is page-local, and says so: a group split across
                // pages must not read as a table total. The header
                // carries an id derived from its label — never from its
                // position — so the in-place morph can follow it.
                row_data[start].group_header = Some(GroupHeader {
                    dom_id: group_header_dom_id(&label),
                    text: format!("{label} ({} on this page)", end - start),
                });
                start = end;
            }
        }
        debug_assert!(
            {
                let mut seen = std::collections::HashSet::new();
                row_data.iter().all(|row| seen.insert(row.key.clone()))
            },
            "duplicate Table::id keys in one page: Table::id must be injective"
        );
        row_data
    }

    /// The fail-visible filter banner: requested filters that produced
    /// no predicate render as a `role=alert` banner; the list keeps a 200 while
    /// the export refuses with 400 (see `resource_export`).
    ///
    /// No false tail: when other filters still apply, "unfiltered" would be a
    /// lie — a malformed segment can ride alongside valid ones.
    /// Conversely an invalid-only request applies nothing, so "other filter(s)"
    /// would be the lie — the consequence keys off applied
    /// predicates, not raw entries.
    fn render_filter_warning<'a>(
        &self,
        cx: &'a Cx,
        state: &NormalizedState,
        path: &str,
    ) -> Option<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let unapplied = self.unapplied_filters(state);
        if unapplied.is_empty() {
            return None;
        }
        let detail = unapplied
            .iter()
            .map(|(pair, reason)| format!("{pair} ({reason})"))
            .collect::<Vec<_>>()
            .join(", ");
        let consequence = if self.filter_expr(state).is_none() {
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

    /// The row-delete confirmation dialog, rendered with the table when
    /// [`Self::with_delete`] wired the delete route.
    ///
    /// One dialog per table: the row Delete controls name it
    /// (`data-row-delete-trigger`) and carry the record's POST target
    /// (`data-row-delete-action`), which `assets/dialog.js` writes to the form
    /// before opening it in place. The control keeps its `?delete=<row key>`
    /// href, so a page without the script opens the dialog through the URL —
    /// and that render ships it open. `?open=false`, the mirror `dialog.js`
    /// writes on dismissal ([`TableState::open`]), leaves it closed. Cancel is a
    /// `data-dialog-close` button on both paths, so dismissal never navigates.
    ///
    /// [`Self::render_with_state`] renders it with the table; the live-search
    /// page (`panel::resource_list_live`) calls this separately because the
    /// shard swaps the table per keystroke and must not carry dialog state.
    ///
    /// Escape/backdrop dismissal, the `data-dialog-close` cancel hook and the
    /// trigger wiring need `assets/dialog.js` (`tablo_ui::DIALOG_JS`),
    /// emitted by `Panel::render_document` on every document with shell assets
    /// (ADR-0014). Without the document scripts Cancel is inert and Delete still
    /// POSTs; the dialog primitives are vendored under the ADR-0007 sync guard,
    /// so they carry no note themselves.
    pub async fn render_delete_dialog<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
    ) -> Result<Option<BoxView<'a>>> {
        self.render_delete_dialog_normalized(cx, &self.normalize_state(state))
            .await
    }

    /// [`Self::render_delete_dialog`] with the state already normalized
    /// `render_inner` and the panel's live page both render the
    /// dialog from the one state the request normalized.
    pub(crate) async fn render_delete_dialog_normalized<'a>(
        &self,
        cx: &'a Cx,
        state: &NormalizedState,
    ) -> Result<Option<BoxView<'a>>> {
        let Some(prefix) = self.delete_prefix.as_deref() else {
            return Ok(None);
        };
        let key = state
            .delete
            .as_deref()
            .filter(|_| state.open != Some(false));
        let server_open = key.is_some();
        let action = key.map(|key| delete_action_url(prefix, key));
        // Only the URL-driven dialog mirrors its dismissal into the URL: a
        // dialog a row control opens client-side has no `?delete=` to close,
        // so dismissing it leaves the URL alone (GH #154 §3).
        let open_param = server_open.then_some("open");
        let dialog_id = Self::delete_dialog_dom_id(prefix);
        let title_id = format!("{dialog_id}-title");
        let description_id = format!("{dialog_id}-description");
        let csrf = crate::csrf::current_token(cx);
        Ok(Some(
            view! {
                cx =>
                alert_dialog(
                    open: server_open,
                    attrs: attributes! {
                        id=(dialog_id)
                        aria-labelledby=(title_id.clone())
                        aria-describedby=(description_id.clone())
                        data-dialog-open-param=(open_param)
                    },
                    dialog_content(
                        dialog_header(
                            dialog_title(
                                attrs: attributes! { id=(title_id.clone()) },
                                "Delete this record?"
                            )
                            dialog_description(
                                attrs: attributes! { id=(description_id.clone()) },
                                "This action cannot be undone."
                            )
                        )
                        dialog_footer(
                            <form
                                method="post"
                                action=(action)
                                class="contents"
                                data-row-delete-form=""
                                data-mutation-submit=""
                            >
                                button(
                                    variant: ButtonVariant::Outline,
                                    size: ButtonSize::Md,
                                    attrs: attributes! { type="button" data-dialog-close="" },
                                    "Cancel"
                                )
                                <input type="hidden" name="confirm" value="1">
                                (crate::csrf::field(cx, &csrf))
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

    /// The DOM id of a table's row-delete dialog: the delete prefix
    /// with its slashes flattened, so two tables with different delete prefixes
    /// never share an id. Two tables over one prefix (a page rendering the same
    /// resource twice) derive the same ids; the panel renders one list table
    /// per page — the list parameters are shared — so its own routes cannot
    /// reach that. The row controls name the dialog they open, and its
    /// `aria-labelledby`/`aria-describedby` ids derive from it.
    fn delete_dialog_dom_id(prefix: &str) -> String {
        format!("{}-delete-dialog", prefix.replace('/', "-"))
    }

    /// The skeleton placeholder table — three pulsing rows under the real
    /// column header. This is the [`suspense`] fallback for tables whose rows
    /// stream in. Wrapped in the same `data-boundary` region as the real table
    /// so the markup shape matches when the swap arrives.
    /// Carries `aria-busy` while loading plus toolbar/pager pulse placeholders
    /// so the streamed chrome lands without a layout shift.
    pub async fn render_skeleton<'a>(&self, cx: &'a Cx) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let state = TableState::from_cx(cx);
        // Same normalization as the table seams: the placeholder
        // header links must not echo an unknown `?group_by=`.
        self.render_skeleton_normalized(cx, &self.normalize_state(&state))
            .await
    }

    /// [`Self::render_skeleton`] with the state already normalized:
    /// the panel parses and normalizes once per request and renders the
    /// streamed placeholder from that same state.
    pub(crate) async fn render_skeleton_normalized<'a>(
        &self,
        cx: &'a Cx,
        state: &NormalizedState,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        let path = topcoat::context::try_request_context::<http::request::Parts>(cx)
            .map(|parts| parts.uri.path().to_string())
            .unwrap_or_default();
        // The action column exists for any of the three row links, matching
        // `render_inner` — a `with_view`-only table must not swap a
        // narrower skeleton for a wider table.
        let with_actions = self.with_actions();
        let with_bulk = self.bulk_enabled();
        let head = self
            .render_thead(cx, state, &path, with_actions, with_bulk, None)
            .await?;
        // The same floor the loaded table carries, so the swap lands without
        // a layout shift.
        let table_min_width = self.column_widths().table_min_width;
        let column_count = self.columns.len();
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
                    attrs: attributes! { class="table-fixed" style=(table_min_width.as_deref()) },
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
                                if with_actions {
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
        // The busy state rides on the morph boundary so assistive
        // tech sees the live region, not just the swapped root below it.
        Ok(view! { cx => <div data-boundary="table" aria-busy="true">(inner)</div> }.boxed())
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
        let dir_hidden = state.sort.as_ref().map(|s| {
            if s.descending {
                "desc".to_string()
            } else {
                "asc".to_string()
            }
        });
        let filters_hidden = state.filters_param();
        // Pre-normalized by the render seams: `state.group_by` is
        // the declared name or `None`, never an unknown value.
        let group_hidden = state.group_by.clone();
        // Clear only renders when something survives the search term; every
        // branch below projects the same URL, so one intent serves all three.
        let clear_url =
            (state.sort.is_some() || filters_hidden.is_some() || group_hidden.is_some())
                .then(|| state.without_search(path));
        let hidden = hidden_state_inputs(
            cx,
            vec![
                ("sort", sort_hidden),
                ("dir", dir_hidden),
                ("filters", filters_hidden),
                ("group_by", group_hidden),
            ],
        );
        Ok(view! {
            cx =>
            <form
                method="get"
                action=(action)
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
            >
                (hidden)
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

    /// Eager live-search input for live tables: the signal-backed
    /// input plus the GET form as `<noscript>` fallback. Rendered eagerly
    /// above the streamed region; the shard invocation that fills the table
    /// lives in the streamed region (`Self::render_live_invocation`) so the
    /// table can only ever render once per response.
    ///
    /// The visible input is deliberately unbound: typing stays
    /// local until it pauses for `LIVE_SEARCH_DEBOUNCE_MS`, then
    /// `assets/live-search.js` forwards the value through the bound hidden
    /// transport, whose `@change` writes `q` and clears the cursors (a new
    /// term is a new result set). The shard re-renders in place.
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
        // normalize for the `<noscript>` fallback links.
        self.render_live_search_bar_normalized(cx, &self.normalize_state(state), path, signals)
            .await
    }

    /// [`Self::render_live_search_bar`] with the state already normalized
    /// the panel's live page renders the toolbar from the request's
    /// one normalized state.
    pub(crate) async fn render_live_search_bar_normalized<'a>(
        &self,
        cx: &'a Cx,
        state: &NormalizedState,
        path: &str,
        signals: &TableSignals,
    ) -> Result<BoxView<'a>> {
        let fallback = self.render_search_bar(cx, state, path).await?;
        let q_display = state.search.clone().unwrap_or_default();
        let q = signals.q.clone();
        let cursor = signals.cursor.clone();
        let none = crate::resource::cursor_none();
        Ok(view! {
            cx =>
            <div
                class="flex flex-wrap items-center gap-2 border-b border-border p-3"
                data-live-search=""
            >
                <input
                    type="search"
                    value=(q_display)
                    placeholder="Search…"
                    aria-label="Live search table"
                    class="w-64"
                    data-live-search-input=""
                    data-debounce-ms=(LIVE_SEARCH_DEBOUNCE_MS)
                >
                <input
                    type="hidden"
                    :value=$(q.get())
                    @change=$(|e: Event| {
                        q.set(e.target.value);
                        cursor.set(none.clone());
                    })
                    data-live-search-transport=""
                >
                <noscript>(fallback)</noscript>
            </div>
        }
        .boxed())
    }

    /// The `table_search` shard invocation filling a live table's streamed
    /// region. The signal handles travel as arguments; every
    /// tracked read inside the shard becomes a `dep` marker the browser
    /// watches, so sort/filter/pager/search changes re-render the table in
    /// place.
    pub(crate) async fn render_live_invocation<'a>(
        &self,
        cx: &'a Cx,
        path: &str,
        signals: TableSignals,
    ) -> Result<BoxView<'a>> {
        use crate::panel::table_search;

        // No snapshot here: grouping travels as the `group_by`
        // live signal (seeded from the page state by the caller) and the
        // shard normalizes on read.
        let live_path = path.to_string();
        let TableSignals {
            q,
            filters,
            sort,
            dir,
            cursor,
            group_by,
            bulk,
        } = signals;
        Ok(view! {
            cx =>
            table_search(
                path: $(live_path.clone()),
                q: $(q),
                filters: $(filters),
                sort: $(sort),
                dir: $(dir),
                cursor: $(cursor),
                group_by: $(group_by),
                bulk: $(bulk)
            )
        }
        .boxed())
    }

    /// The filter bar for a live table, rendered eagerly by the page that owns
    /// the signals — the counterpart of [`Self::render_live_search_bar`].
    ///
    /// Hoisting matters for focus: a `<select>` change writes the `filters`
    /// signal, and a bar rebuilt by that rerun would collapse the native popup
    /// and drop keyboard context. The table renders without the bar
    /// (`Table::filter_bar(false)`), so the control the user touched is never
    /// replaced.
    pub async fn render_live_filter_bar<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        signals: &TableSignals,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        // Called with raw page state: normalize so the no-JS
        // fallback form carries the same normalized values the GET path would.
        self.render_live_filter_bar_normalized(cx, &self.normalize_state(state), path, signals)
            .await
    }

    /// [`Self::render_live_filter_bar`] with the state already normalized
    /// the panel's live page renders the hoisted bar from the
    /// request's one normalized state.
    pub(crate) async fn render_live_filter_bar_normalized<'a>(
        &self,
        cx: &'a Cx,
        state: &NormalizedState,
        path: &str,
        signals: &TableSignals,
    ) -> Result<BoxView<'a>>
    where
        M: toasty::schema::Model,
    {
        self.render_filter_bar(cx, state, path, Some(signals)).await
    }

    /// The typed filter bar. For live tables (`signals`) the hidden `filters`
    /// transport is bound to the `filters` signal and `filters.js` dispatches
    /// a `change` into it instead of submitting, so the shard re-renders the
    /// table in place; the GET form stays as the no-JS fallback and `href`s
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
        // No `filters.is_empty()` early return: the caller (`render_inner`)
        // already guards on `show_filters`, so an empty bar is unreachable.
        let action = path.to_string();
        let filters_display = state.filters_param().unwrap_or_default();
        let sort_hidden = state.sort.as_ref().map(|s| s.column.clone());
        let dir_hidden = state.sort.as_ref().map(|s| {
            if s.descending {
                "desc".to_string()
            } else {
                "asc".to_string()
            }
        });
        let q_hidden = state.search.clone();
        let group_hidden = state.group_by.clone();
        let hidden = hidden_state_inputs(
            cx,
            vec![
                ("q", q_hidden),
                ("sort", sort_hidden),
                ("dir", dir_hidden),
                ("group_by", group_hidden),
            ],
        );
        let clear_url = if !state.filters.is_empty() {
            Some(state.without_filters(path))
        } else {
            None
        };
        // One typed control per declared filter. Controls carry only
        // `data-filter-name` (no `name`, so they never submit on their own);
        // `filters.js` composes them into the hidden `filters` transport and
        // submits on change, rewriting it even when every control is
        // "All" so the stale value can never be resubmitted. The free-text
        // input and Apply button survive only inside `<noscript>` as the
        // no-JS fallback.
        let mut controls: Vec<BoxView<'_>> = Vec::with_capacity(self.filters.len());
        for f in &self.filters {
            let current = state.filters.get(f.name()).cloned().unwrap_or_default();
            match f {
                Filter::Select(s) => {
                    // A declared empty option is the clear-filter value, so it
                    // renders as the "All" option: value `""`, label "All".
                    let options = s
                        .options()
                        .iter()
                        .map(|opt| {
                            let label = if opt.is_empty() { "All" } else { opt.as_str() };
                            (opt.clone(), label.to_string())
                        })
                        .collect();
                    controls.push(filter_select(
                        cx,
                        s.label_str(),
                        s.name(),
                        options,
                        &current,
                    ));
                }
                Filter::Ternary(t) => {
                    let options = vec![
                        ("true".to_string(), "True".to_string()),
                        ("false".to_string(), "False".to_string()),
                    ];
                    controls.push(filter_select(
                        cx,
                        t.label_str(),
                        t.name(),
                        options,
                        &current,
                    ));
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
                    let options = v
                        .options()
                        .iter()
                        .map(|(key, _)| (key.clone(), key.clone()))
                        .collect();
                    controls.push(filter_select(
                        cx,
                        v.label_str(),
                        v.name(),
                        options,
                        &current,
                    ));
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
            let (filters, cursor) = (signals.filters.clone(), signals.cursor.clone());
            let none = crate::resource::cursor_none();
            attributes! {
                cx =>
                name="filters"
                :value=$(filters.get())
                @change=$(|e: Event| {
                    filters.set(e.target.value);
                    cursor.set(none.clone());
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
                    let (filters, cursor) = (signals.filters.clone(), signals.cursor.clone());
                    let none = crate::resource::cursor_none();
                    attributes! {
                        cx =>
                        href=(url.clone())
                        @click=$(|e: Event| {
                            e.prevent_default();
                            filters.set("".to_owned());
                            cursor.set(none.clone());
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
                (hidden)
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
    /// the table. For live tables (`signals`) the clear/back links write the
    /// signals instead of navigating; `href` stays the fallback.
    async fn render_empty_cell<'a>(
        &self,
        cx: &'a Cx,
        state: &TableState,
        path: &str,
        with_actions: bool,
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
        if with_actions {
            colspan += 1;
        }
        let filtered = state.search.is_some() || !state.filters.is_empty();
        // Clear only the dimension the link names and keep the rest of the
        // state (follow-up): the URL is rebuilt from the full state, so
        // `group_by` survives, and a "Clear search" link leaves the filters
        // alone.
        let clear_url = filtered.then(|| {
            if state.search.is_some() {
                state.without_search(path)
            } else {
                state.without_filters(path)
            }
        });
        // Search matches anywhere in the value, so the empty copy
        // says "matches", not "prefix matches".
        let message = match &state.search {
            Some(term) => format!("No matches for \u{201c}{term}\u{201d}"),
            None if !state.filters.is_empty() => "No results for these filters".to_string(),
            None => "No records yet".to_string(),
        };
        let clear_label = if state.search.is_some() {
            "Clear search"
        } else {
            "Clear filters"
        };
        // Void window: a cursor that lands past the last row (e.g.
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
                    let none = crate::resource::cursor_none();
                    let (q, filters, cursor) = (
                        signals.q.clone(),
                        signals.filters.clone(),
                        signals.cursor.clone(),
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
                            cursor.set(none.clone());
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
                    let cursor = signals.cursor.clone();
                    let none = crate::resource::cursor_none();
                    attributes! {
                        cx =>
                        href=(url)
                        @click=$(|e: Event| {
                            e.prevent_default();
                            cursor.set(none.clone());
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
        if self.page_size.is_none() {
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
                    let wire = crate::resource::cursor_before(cursor);
                    let signal = signals.cursor.clone();
                    attributes! {
                        cx =>
                        href=(href.clone())
                        @click=$(|e: Event| {
                            e.prevent_default();
                            signal.set(wire.clone());
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
                    let wire = crate::resource::cursor_after(cursor);
                    let signal = signals.cursor.clone();
                    attributes! {
                        cx =>
                        href=(href.clone())
                        @click=$(|e: Event| {
                            e.prevent_default();
                            signal.set(wire.clone());
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

    /// Whether the table renders a row-actions column.
    fn with_actions(&self) -> bool {
        self.delete_prefix.is_some() || self.edit_prefix.is_some() || self.view_prefix.is_some()
    }

    /// How many row links sit side by side in the actions column.
    fn action_link_count(&self) -> usize {
        usize::from(self.view_prefix.is_some())
            + usize::from(self.edit_prefix.is_some())
            + usize::from(self.delete_prefix.is_some())
    }

    /// The share of the table the row-actions column claims: the row
    /// links sit side by side and each is a fixed-size control, so the share
    /// grows with the number of links the table renders. The values hold the
    /// widest set at a 1280px window and the narrower sets inside it.
    fn actions_percent(&self) -> u8 {
        match self.action_link_count() {
            2 => 18,
            3.. => 25,
            _ => 12,
        }
    }

    /// The content floor of the row-actions column, in whole rem: one row of
    /// `Md` buttons plus the cell's `p-3` padding, by link count. The share
    /// above is a fraction of the table and shrinks with it, so on a narrow
    /// viewport the buttons would spill past the table and clip against the
    /// chrome's `overflow-hidden`; the floor keeps the column as wide as its
    /// buttons, and the table's `min-width` keeps the table as wide as its
    /// columns, so the wrapper scrolls instead.
    fn actions_min_rem(&self) -> u8 {
        match self.action_link_count() {
            2 => 11,
            3.. => 15,
            _ => 7,
        }
    }

    /// The width every column of this table declares.
    ///
    /// The kind defaults — a [`ColumnWidth::Narrow`] column, the bulk
    /// checkbox, the row actions — are shares of the table, scaled down
    /// together when their nominal total exceeds
    /// [`DEFAULT_WIDTH_BUDGET_PERCENT`]: the wide columns take what the
    /// declared ones leave, and a table that spends every percent on declared
    /// columns leaves them none. An explicit `Rem`/`Percent` is emitted as
    /// declared.
    ///
    /// The table-level `min-width` is the sum of those declarations: every
    /// share as emitted, every `Rem` verbatim, the actions column's content
    /// floor, and one [`WIDE_COLUMN_MIN_REM`] per wide column (which declares
    /// nothing and would otherwise crush to zero). With `w-full` the table
    /// never exceeds its container on its own, so without the floor the
    /// wrapper's `overflow-x-auto` never scrolls; with it the table keeps its
    /// measure on a narrow viewport and the wrapper scrolls. Emitted only
    /// when the sum carries a length — shares alone are a fraction of the
    /// container and can never overflow it.
    fn column_widths(&self) -> ColumnWidths
    where
        M: toasty::schema::Model,
    {
        let bulk = self.bulk_enabled().then_some(BULK_COLUMN_PERCENT);
        let actions = self.with_actions().then(|| self.actions_percent());
        let actions_floor = self.with_actions().then(|| self.actions_min_rem());
        let total: u32 = bulk
            .into_iter()
            .chain(
                self.columns
                    .iter()
                    .filter_map(|col| col.column_width().default_percent()),
            )
            .chain(actions)
            .map(u32::from)
            .sum();
        let default_style =
            |nominal: u8| default_width_style(scaled_default_percent(nominal, total));

        let cells = self
            .columns
            .iter()
            .map(|col| {
                let width = col.column_width();
                // `Wide` declares nothing; a kind default is resolved against
                // the rest of the table; `Rem`/`Percent` are verbatim.
                width
                    .explicit_css()
                    .or_else(|| width.default_percent().map(default_style))
            })
            .collect();
        // The `min-width` terms, in layout order: the shares first, then the
        // lengths as one rem total. A scaled share is the emitted one, so the
        // floor and the column agree.
        let mut percent_terms: Vec<u8> = Vec::new();
        if let Some(share) = bulk {
            percent_terms.push(scaled_default_percent(share, total));
        }
        let mut rem_total: u32 = 0;
        for col in &self.columns {
            match col.column_width() {
                ColumnWidth::Wide => rem_total += u32::from(WIDE_COLUMN_MIN_REM),
                ColumnWidth::Narrow => {
                    percent_terms.push(scaled_default_percent(NARROW_DEFAULT_PERCENT, total));
                }
                ColumnWidth::Rem(rem) => rem_total += u32::from(rem),
                ColumnWidth::Percent(share) => percent_terms.push(share),
            }
        }
        let mut actions_style = None;
        if let (Some(share), Some(floor)) = (actions, actions_floor) {
            let scaled = scaled_default_percent(share, total);
            percent_terms.push(scaled);
            rem_total += u32::from(floor);
            actions_style = Some(Cow::Owned(format!(
                "width: {scaled}%; min-width: {floor}rem"
            )));
        }
        let table_min_width = (rem_total > 0).then(|| {
            let mut parts: Vec<String> = percent_terms
                .iter()
                .map(|share| format!("{share}%"))
                .collect();
            parts.push(format!("{rem_total}rem"));
            if parts.len() == 1 {
                Cow::Owned(format!("min-width: {}", parts[0]))
            } else {
                Cow::Owned(format!("min-width: calc({})", parts.join(" + ")))
            }
        });
        ColumnWidths {
            cells,
            bulk: bulk.map(default_style),
            actions: actions_style,
            actions_min: actions_floor.map(|floor| Cow::Owned(format!("min-width: {floor}rem"))),
            table_min_width,
        }
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
        with_actions: bool,
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
        let widths = self.column_widths();
        let mut heads: Vec<BoxView<'_>> = Vec::with_capacity(self.columns.len());
        for (index, col) in self.columns.iter().enumerate() {
            // Owned per iteration: the view must not borrow the resolved list.
            let width = widths.cells[index].clone();
            let label = col.label().to_string();
            // The declared width rides the header cell's inline `style`
            // a Tailwind class assembled at render would emit no
            // CSS, because Tailwind only generates the literals it finds in
            // source. A wide column declares nothing and takes a share of what
            // the declared columns leave.
            // A static preview renders plain labels: no link to an interaction
            // the page does not honor.
            let sortable = col.is_sortable();
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
                    let none = crate::resource::cursor_none();
                    let (sort, dir, cursor) = (
                        signals.sort.clone(),
                        signals.dir.clone(),
                        signals.cursor.clone(),
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
                            cursor.set(none.clone());
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
                        attrs: attributes! {
                            class=(head_class)
                            aria-sort=(aria_sort)
                            style=(width.as_deref())
                        },
                        (header)
                    )
                }
                .boxed(),
            );
        }
        if with_actions {
            heads.push(
                view! {
                    cx =>
                    // The share sizes the column at wide viewports; the floor
                    // sizes it to its buttons at narrow ones, where the share
                    // alone would let them spill past the table. Each row's
                    // `td` repeats only the floor.
                    table_head(
                        attrs: attributes! { style=(widths.actions.as_deref()) },
                        "Actions"
                    )
                }
                .boxed(),
            );
        }
        Ok(view! {
            cx =>
            table_header(
                table_row(
                    if with_bulk {
                        // The header row is the row `table-fixed` measures, so
                        // the chrome columns declare their width here and
                        // their `td`s declare none.
                        table_head(
                            attrs: attributes! { style=(widths.bulk.as_deref()) },
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

/// Precomputed per-row presentation for the table body: the display row key,
/// the record key, the rendered cells, and the optional Edit / delete-dialog
/// action URLs. A struct (not a tuple): five anonymous positions would
/// mislead readers and trip `clippy::type_complexity`.
///
/// `key` is the `Table::id` display projection (keyed diffs, DOM ids);
/// `record_id` is the `Table::pk` projection (URLs, bulk values), resolved
/// by handlers as the typed PK.
struct RowView {
    key: String,
    record_id: String,
    cells: Vec<String>,
    view_url: Option<String>,
    edit_url: Option<String>,
    delete_url: Option<String>,
    /// The row's delete POST target (`{prefix}/{key}/delete`): the
    /// Delete control hands it to the shared dialog before opening it, so the
    /// confirmed POST keeps the route the `?delete=` fallback uses.
    delete_action: Option<String>,
    /// Whether the row's bulk checkbox is enabled: a row the
    /// [`Table::row_actions`] policy denies `delete` renders it `disabled`, so
    /// `bulk.js` never lets it into the selection transport and select-all
    /// cannot submit a batch the handler refuses wholesale.
    selectable: bool,
    /// Whether the row's policy allows no action at all: the template renders
    /// the [`LOCKED_ROW_LABEL`] badge in place of the links instead of an
    /// empty cell, so the row still carries the actions column its header
    /// declares.
    locked: bool,
    /// The row's group label, when `?group_by=` named the declared group.
    /// Carried on every row so the page-local shim can order by it.
    group: Option<String>,
    /// The header this row renders above itself, `Some` only on the first row
    /// of its group.
    group_header: Option<GroupHeader>,
}

/// One page-local group header: the label with its page-local count,
/// and the stable DOM id the injected header row carries so the in-place morph
/// can follow it (`row_dom_id`'s contract).
#[derive(Clone)]
struct GroupHeader {
    /// `"{label} ({n} on this page)"`.
    text: String,
    /// [`group_header_dom_id`] of the label.
    dom_id: String,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use topcoat::context::CxTestBuilder;

    use super::*;
    use crate::resource::{
        ColumnWidth, DateFilter, SelectFilter, Sort, TernaryFilter, TextColumn, VariantFilter,
    };

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
        VariantFilter::r#for(
            "vehicule",
            "Véhicule",
            vec![
                ("Auto".to_string(), Driver::fields().vehicule().is_auto()),
                ("Moto".to_string(), Driver::fields().vehicule().is_moto()),
            ],
        )
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

    fn last_link_named<'a>(html: &'a str, label: &str) -> &'a str {
        html.rsplit('<')
            .find(|chunk| chunk.contains(label))
            .unwrap_or_else(|| panic!("missing {label} link in {html}"))
    }

    /// The `<table …>` opening tag of a rendered table, without its children.
    fn table_tag(html: &str) -> &str {
        let start = html.find("<table").expect("the table element");
        let end = html[start..].find('>').expect("its tag end") + start;
        &html[start..end]
    }

    /// The layout a `<table ...>` tag declares, independent of attribute
    /// order: the sorted `class`/`style` values the tag carries. Two tags
    /// declaring the same layout compare equal even when the serializer
    /// emits `style` before `class` in one and after it in the other.
    fn normalized_table_tag(tag: &str) -> (String, String) {
        (table_attr(tag, "class"), table_attr(tag, "style"))
    }

    /// The value of one quoted attribute inside a tag, or empty when absent.
    fn table_attr(tag: &str, name: &str) -> String {
        let marker = format!("{name}=\"");
        let Some(at) = tag.find(&marker) else {
            return String::new();
        };
        let rest = &tag[at + marker.len()..];
        let mut classes: Vec<&str> = rest
            .split('"')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        classes.sort_unstable();
        classes.join(" ")
    }

    /// Every whole-percent width a rendered table declares, in document order.
    /// A length declaration is skipped: those carry a unit. A share paired
    /// with a content floor (`width: 18%; min-width: 11rem`) still parses: the
    /// share ends at the `;`, not at the attribute's closing quote.
    fn declared_percents(html: &str) -> Vec<u32> {
        html.match_indices("style=\"width: ")
            .filter_map(|(at, marker)| {
                html[at + marker.len()..]
                    .split(['"', ';'])
                    .next()?
                    .strip_suffix('%')?
                    .parse()
                    .ok()
            })
            .collect()
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
        // Columns but no row key → error
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
        // Zero page size is a programmer error: a descriptive error
        // the streamed list renders in-region, never a per-request panic.
        let zero = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
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

    #[test]
    fn live_search_debounce_sits_in_the_locked_band() {
        // GH #172 decision 4: ~150-250ms at the `@input` handler. The
        // markup test below pins the rendered value; this pins the range.
        assert!(
            (150..=250).contains(&LIVE_SEARCH_DEBOUNCE_MS),
            "debounce must sit in the 150-250ms band, got {LIVE_SEARCH_DEBOUNCE_MS}"
        );
    }

    #[tokio::test]
    async fn table_for_columns_renders_with_keyed_rows() {
        let cx = CxTestBuilder::new().build();
        // columns need distinct names — title + status, not one
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
        // no Tailwind-class assertions. The chrome literals
        // (`rounded-xl`, `border-border`, `text-muted-foreground`,
        // `cursor-pointer`) are the showcase's business (#136), and pinning
        // them here meant every restyle broke a core test.
        //
        // Searchable columns render no extra header chrome: the
        // search input is the affordance, so the header cell holds its label
        // and nothing interactive. The sortable sibling next door *does* carry
        // an `<a>` and an icon, so this can fail.
        let title_at = html.find("Title").expect("the Title header");
        let title_th = html[..title_at].rfind("<th").expect("its <th>");
        let title_th_end = html[title_th..].find("</th>").expect("its </th>") + title_th;
        let title_head = &html[title_th..title_th_end];
        assert!(
            !title_head.contains("<svg") && !title_head.contains("<a "),
            "a searchable header must render no sort or loupe chrome, got {title_head}"
        );
        // Sortable ones carry the inactive `arrow-up-down` with
        // `aria-sort="none"`.
        assert!(
            html.contains("aria-sort=\"none\""),
            "missing sortable indicator in {html}"
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

    /// the table lays out fixed, and a declared column width reaches
    /// the header cell and every row's cell as data — an inline `style`, never
    /// a Tailwind class built at render.
    #[tokio::test]
    async fn table_lays_out_fixed_and_emits_declared_column_widths() {
        let cx = CxTestBuilder::new().build();
        let width_table = Table::<Task>::r#for(&cx).id(|t| t.id.to_string()).columns((
            // A field-backed column defaults to `Wide`: it declares no
            // width and takes the share the declared columns leave.
            TextColumn::r#for(Task::fields().title(), |t: &Task| t.title.clone()),
            // A computed column defaults to `Narrow`, overridden here.
            TextColumn::computed("Status", |t: &Task| t.status.clone())
                .width(ColumnWidth::Percent(30)),
        ));
        let page: TablePage<Task> = vec![Task {
            id: uuid::Uuid::new_v4(),
            title: "Ada".to_string(),
            status: "draft".to_string(),
            featured: false,
            created_at: jiff::Timestamp::now(),
        }]
        .into();
        let html = width_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // The fixed layout is the table's own contract, not paint: the
        // Done-when names it as the observable and a class is its only
        // transport, so this is the one class literal asserted here. The paint
        // classes stay the showcase's business (#136).
        let tag = table_tag(&html);
        assert!(
            tag.contains("table-fixed"),
            "the table must lay out fixed, got {tag}"
        );
        // The declared width is data on the header and on the row's cell: one
        // declaration, two carriers.
        assert_eq!(
            html.matches("style=\"width: 30%\"").count(),
            2,
            "the declared width must reach the th and the td, got {html}"
        );
        // The wide column declares nothing: an absent attribute, not a
        // generated class.
        assert_eq!(
            html.matches("style=\"width").count(),
            2,
            "only the declared column carries a width, got {html}"
        );
    }

    /// a column that declares nothing but its kind claims a share of
    /// the table — a percentage, so it shrinks with the table instead of
    /// outgrowing it — and the wide column beside it still declares none.
    #[tokio::test]
    async fn kind_defaults_claim_a_share_of_the_table() {
        let cx = CxTestBuilder::new().build();
        let default_table = Table::<Task>::r#for(&cx).id(|t| t.id.to_string()).columns((
            TextColumn::r#for(Task::fields().title(), |t: &Task| t.title.clone()),
            TextColumn::computed("Status", |t: &Task| t.status.clone()),
        ));
        let page: TablePage<Task> = vec![Task {
            id: uuid::Uuid::new_v4(),
            title: "Ada".to_string(),
            status: "draft".to_string(),
            featured: false,
            created_at: jiff::Timestamp::now(),
        }]
        .into();
        let html = default_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert_eq!(
            declared_percents(&html),
            [10, 10],
            "a computed column claims its kind's share on the th and the td, got {html}"
        );
        assert_eq!(
            html.matches("style=\"width").count(),
            2,
            "the field column must declare nothing, got {html}"
        );
    }

    /// the chrome columns declare a share of the table too — the
    /// header row is the row `table-fixed` measures — and the share grows with
    /// the number of row links, which sit side by side. The actions column
    /// pairs its share with a content floor (`min-width: 11rem`), on the
    /// header and on every row's cell, so the buttons fit instead of spilling
    /// past the table on a narrow viewport.
    #[tokio::test]
    async fn chrome_columns_declare_their_widths() {
        // Each case: the row links to wire, the share Actions claims, and the
        // floor that holds its buttons.
        let cases: [(usize, &str, &str); 3] =
            [(1, "12%", "7rem"), (2, "18%", "11rem"), (3, "25%", "15rem")];
        for (links, expected, floor) in cases {
            let cx = CxTestBuilder::new().build();
            let mut chrome_table = Table::<User>::r#for(&cx)
                .id(|u| u.id.to_string())
                .pk(|u| u.id.to_string())
                .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
                .with_view("/admin/users".to_string());
            if links > 1 {
                chrome_table = chrome_table.with_edit("/admin/users".to_string());
            }
            if links > 2 {
                // Delete is what the bulk column pairs with.
                chrome_table = chrome_table
                    .with_delete("/admin/users".to_string())
                    .with_bulk_delete(true);
            }
            let page: TablePage<User> = vec![User {
                id: uuid::Uuid::new_v4(),
                name: "Ada".to_string(),
            }]
            .into();
            let html = chrome_table
                .render(&cx, page)
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(&cx);
            assert_eq!(
                html.matches(&format!("width: {expected}")).count(),
                1,
                "{links} row links must claim {expected} in the header row, got {html}"
            );
            // The floor rides the header and every row's cell, so the buttons
            // fit whatever the share shrinks to.
            assert_eq!(
                html.matches(&format!("min-width: {floor}")).count(),
                2,
                "{links} row links must floor the actions column at {floor}, got {html}"
            );
            // The bulk checkbox claims its own share, and only when the table
            // renders one.
            let bulk = if links > 2 { 1 } else { 0 };
            assert_eq!(
                html.matches(&format!("style=\"width: {BULK_COLUMN_PERCENT}%\""))
                    .count(),
                bulk,
                "the bulk column's share must follow the table's chrome, got {html}"
            );
        }
    }

    /// the kind defaults together stay inside their budget, whatever
    /// the column set — a column that declares none is rendered at zero width
    /// once the declared shares claim the whole table, header text included,
    /// so the defaults scale down instead of spending the last percent.
    #[tokio::test]
    async fn kind_defaults_stay_inside_their_budget() {
        let cx = CxTestBuilder::new().build();
        // Four computed columns (4 × the 10% nominal) plus both chrome columns
        // (5% + 20%) overrun the budget, so every default is scaled down
        // together and the field column beside them keeps the rest.
        let crowded = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .pk(|t| t.id.to_string())
            .columns((
                TextColumn::r#for(Task::fields().title(), |t: &Task| t.title.clone()),
                TextColumn::computed("Status", |t: &Task| t.status.clone()),
                TextColumn::computed("Featured", |t: &Task| t.featured.to_string()),
                TextColumn::computed("Created", |t: &Task| t.created_at.to_string()),
                TextColumn::computed("Id", |t: &Task| t.id.to_string()),
            ))
            .with_delete("/admin/tasks".to_string())
            .with_edit("/admin/tasks".to_string())
            .with_view("/admin/tasks".to_string())
            .with_bulk_delete(true);
        let page: TablePage<Task> = vec![Task {
            id: uuid::Uuid::new_v4(),
            title: "Ada".to_string(),
            status: "draft".to_string(),
            featured: false,
            created_at: jiff::Timestamp::now(),
        }]
        .into();
        let html = crowded
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // One share per declared column, in the header row: the four computed
        // columns and the two chrome columns.
        let thead_at = html.find("<thead").expect("a header row");
        let thead_end = html.find("</thead>").expect("its end");
        let percents = declared_percents(&html[thead_at..thead_end]);
        assert_eq!(
            percents.len(),
            6,
            "one share per declared column, got {percents:?} in {html}"
        );
        assert!(
            percents.iter().all(|percent| *percent > 0),
            "a scaled share must keep its column visible, got {percents:?}"
        );
        let total: u32 = percents.iter().sum();
        assert!(
            total <= u32::from(DEFAULT_WIDTH_BUDGET_PERCENT),
            "the kind defaults must leave the field column a share, got {percents:?}"
        );
        // The field column declares nothing at all, so it takes what the
        // declared columns leave.
        let title_at = html.find(">Title<").expect("the Title header");
        let title_th = html[..title_at].rfind("<th").expect("its <th>");
        assert!(
            !html[title_th..title_at].contains("style="),
            "the field column must declare no width, got {}",
            &html[title_th..title_at]
        );
        // Every share rides its header cell, and each text column repeats its
        // own on the row's cell: four text columns twice, two chrome once.
        assert_eq!(
            declared_percents(&html).len(),
            10,
            "each share must reach its th and its td, got {html}"
        );
    }

    #[tokio::test]
    async fn edit_links_render_beside_delete_in_actions_column() {
        // GH #162 (Filament's `recordActions` EditAction): `with_edit` wires
        // one `Edit` link per row into the shared Actions column.
        let cx = CxTestBuilder::new().build();
        let action_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_delete("/admin/users".to_string())
            .with_edit("/admin/users".to_string());
        let rows = vec![User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        }];
        let id = rows[0].id.to_string();
        let page: TablePage<User> = rows.into();
        let html = action_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Actions"), "missing Actions header in {html}");
        assert!(
            html.contains(&format!("href=\"/admin/users/{id}/edit\"")) && html.contains(">Edit<"),
            "missing Edit link for {id} in {html}"
        );
        assert!(
            html.contains("Delete"),
            "Delete link must survive, got {html}"
        );
        // Without either prefix there is no Actions column at all — and a
        // chromeless table needs no `pk`: nothing emits URLs.
        let plain = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let page: TablePage<User> = vec![User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        }]
        .into();
        let html = plain
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("Actions") && !html.contains(">Edit<"),
            "plain table must not render action chrome, got {html}"
        );
    }

    #[tokio::test]
    async fn bulk_checkboxes_render_with_keys_and_select_all() {
        let cx = CxTestBuilder::new().build();
        let bulk_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
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
        // Per-row checkbox carries the record key; header select-all present.
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
        // Bulk form keeps the hidden `ids` transport and a submit
        // that ships disabled until `bulk.js` sees a checked row.
        assert!(
            html.contains("data-bulk-form"),
            "missing bulk form in {html}"
        );
        assert!(
            html.contains("name=\"ids\"") && !html.contains("ids comma-separated"),
            "missing hidden ids transport in {html}"
        );
        // the destructive write is gated by the confirmation dialog
        // rather than by a disabled control — the trigger opens it, and the
        // dialog's own submit carries `confirm=1` inside the same form.
        assert!(
            html.contains("data-bulk-confirm-trigger"),
            "missing the bulk confirm trigger in {html}"
        );
        assert!(
            html.contains("data-bulk-confirm-dialog"),
            "missing the bulk confirm dialog in {html}"
        );
        assert!(
            html.contains("name=\"confirm\"") && html.contains("value=\"1\""),
            "the dialog must carry the confirm marker in {html}"
        );
        // Rendered closed: it is opened client-side so that opening it is not
        // a result-set change. Matched as `open="` rather than `open`, because
        // the dialog's class carries Tailwind's `open:` state variants.
        let dialog_at = html
            .find("data-bulk-confirm-dialog")
            .expect("the dialog marker");
        let dialog_tag_start = html[..dialog_at].rfind("<dialog").expect("its <dialog>");
        let dialog_tag_end = html[dialog_tag_start..].find('>').expect("the tag's end");
        let dialog_tag = &html[dialog_tag_start..dialog_tag_start + dialog_tag_end];
        assert!(
            !dialog_tag.contains("open=\""),
            "the bulk confirm dialog must render closed, got {dialog_tag}"
        );
        // `dialog.js` refuses to dismiss an alert dialog on a backdrop
        // click, so the role is the contract that keeps the confirm dialog
        // waiting for an answer rather than treating a stray click as one.
        assert!(
            dialog_tag.contains("role=\"alertdialog\""),
            "the bulk confirm dialog must be an alert dialog, got {dialog_tag}"
        );
        // The dialog is the decision, not decoration: it asks, and
        // it offers a way out that is not deleting. Absorbed from the showcase
        // duplicate so the one test that owns bulk chrome owns all
        // of it.
        assert!(
            html.contains("Delete the selected records?"),
            "the dialog must ask before it deletes, got {html}"
        );
        assert!(
            html.contains("data-dialog-close"),
            "the dialog needs a way out that is not deleting, got {html}"
        );
        // The confirm control rides inside the bulk form, so the confirmed
        // submit ships it with the same payload as the selection: `bulk.js`
        // closes over `trigger.closest('form[data-bulk-form]')`, so a dialog
        // outside the form would be decoration a crafted request skips.
        let form_at = html.find("data-bulk-form").expect("the bulk form");
        assert!(
            form_at < dialog_at,
            "the dialog must live inside the bulk form, got {html}"
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
            .pk(|u| u.id.to_string())
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
    async fn denied_rows_render_no_links_and_a_disabled_checkbox() {
        // the row policy gates the chrome per record, so a row the
        // resource refuses renders no Edit/Delete link and a bulk checkbox a
        // user cannot check — the rendered affordance and the route agree.
        let cx = CxTestBuilder::new().build();
        let ada = User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        };
        let ken = User {
            id: uuid::Uuid::new_v4(),
            name: "Ken".to_string(),
        };
        let ken_id = ken.id.to_string();
        let ada_id = ada.id.to_string();
        let policy_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_delete("/admin/users".to_string())
            .with_edit("/admin/users".to_string())
            .with_view("/admin/users".to_string())
            .with_bulk_delete(true)
            .row_actions(|u: &User| RowActions {
                view: true,
                edit: u.name != "Ken",
                delete: u.name != "Ken",
            });
        let page: TablePage<User> = vec![ada, ken].into();
        let html = policy_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // The allowed row keeps all three links and an enabled checkbox.
        assert!(
            html.contains(&format!("href=\"/admin/users/{ada_id}/edit\""))
                && html.contains(&format!("href=\"/admin/users/{ada_id}\""))
                && html.contains(&format!("href=\"?delete={ada_id}\"")),
            "the allowed row must keep its View/Edit/Delete links, got {html}"
        );
        // The denied row keeps only the View link its policy allows: no Edit
        // link and no delete dialog opener.
        assert!(
            html.contains(&format!("href=\"/admin/users/{ken_id}\""))
                && !html.contains(&format!("/admin/users/{ken_id}/edit"))
                && !html.contains(&format!("delete={ken_id}")),
            "the denied row must render no Edit/Delete link, got {html}"
        );
        // Its checkbox is present (the transport shape is unchanged) but
        // disabled, carrying the reason as its accessible label.
        let ken_at = html
            .find(&format!("value=\"{ken_id}\""))
            .unwrap_or_else(|| panic!("missing the denied row's checkbox in {html}"));
        let tag_start = html[..ken_at].rfind("<input").expect("its opening tag");
        let tag_end = html[tag_start..].find('>').expect("the tag's end");
        let tag = &html[tag_start..tag_start + tag_end];
        assert!(
            tag.contains("data-row-select"),
            "the denied row keeps the bulk checkbox marker, got {tag}"
        );
        assert!(
            tag.contains("disabled=\"\""),
            "the denied row's checkbox must be disabled, got {tag}"
        );
        assert!(
            tag.contains(&format!("aria-label=\"{DENIED_ROW_REASON}\""))
                && tag.contains(&format!("title=\"{DENIED_ROW_REASON}\"")),
            "the denied row's checkbox must carry the reason, got {tag}"
        );
        // The allowed row's checkbox is not disabled, so the assertion above
        // is not passing on every row.
        let ada_at = html
            .find(&format!("value=\"{ada_id}\""))
            .expect("Ada's box");
        let ada_start = html[..ada_at].rfind("<input").expect("its opening tag");
        let ada_end = html[ada_start..].find('>').expect("the tag's end");
        let ada_tag = &html[ada_start..ada_start + ada_end];
        assert!(
            !ada_tag.contains("disabled") && ada_tag.contains("aria-label=\"Select row\""),
            "the allowed row's checkbox must stay enabled, got {ada_tag}"
        );
    }

    /// The `<tr>…</tr>` chunk holding the row checkbox with `value`, without
    /// its closing tag: the row's own cells scoped down from the page.
    fn row_chunk<'a>(html: &'a str, value: &str) -> &'a str {
        let at = html
            .find(&format!("value=\"{value}\""))
            .unwrap_or_else(|| panic!("missing the row checkbox with value {value}"));
        let start = html[..at].rfind("<tr").expect("the row's opening tag");
        let end = html[at..].find("</tr>").expect("the row's closing tag") + at;
        &html[start..end]
    }

    #[tokio::test]
    async fn fully_locked_rows_keep_their_actions_cell_with_no_links() {
        // A row the policy locks out of every link keeps its actions cell all
        // the same: the cell stays aligned with the header instead of going
        // missing, and the row carries as many cells as the header.
        let cx = CxTestBuilder::new().build();
        let ada = User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        };
        let ken = User {
            id: uuid::Uuid::new_v4(),
            name: "Ken".to_string(),
        };
        let ken_id = ken.id.to_string();
        let ada_id = ada.id.to_string();
        let policy_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_delete("/admin/users".to_string())
            .with_edit("/admin/users".to_string())
            .with_bulk_delete(true)
            .row_actions(|u: &User| {
                let allowed = u.name != "Ken";
                RowActions {
                    view: allowed,
                    edit: allowed,
                    delete: allowed,
                }
            });
        let page: TablePage<User> = vec![ada, ken].into();
        let html = policy_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // The locked row renders no action link at all.
        let ken_row = row_chunk(&html, &ken_id);
        assert!(
            !ken_row.contains("/admin/users/"),
            "the locked row must render no action link at all, got {ken_row}"
        );
        // The allowed row keeps its links, so the absence above is not
        // passing on a page that renders no chrome at all.
        let ada_row = row_chunk(&html, &ada_id);
        assert!(
            ada_row.contains(&format!("/admin/users/{ada_id}/edit")),
            "the allowed row must keep its links, got {ada_row}"
        );
        // Alignment: the locked row carries a cell per header, badge
        // included. Counted on the closing tags: `<thead` itself opens
        // with `<th`.
        let thead_at = html.find("<thead").expect("a header row");
        let thead_end = html.find("</thead>").expect("its end");
        assert_eq!(
            ken_row.matches("</td>").count(),
            html[thead_at..thead_end].matches("</th>").count(),
            "the locked row must carry a cell per header, got {ken_row}"
        );
    }

    #[tokio::test]
    async fn a_chromeless_table_never_consults_the_row_policy() {
        // the policy is consulted only where chrome is wired, so a
        // resource that declares no chrome keeps its list page free of
        // per-record predicate calls — the coarse `TableChrome` gate is intact.
        let cx = CxTestBuilder::new().build();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let policy_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .row_actions(move |_: &User| {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                RowActions::ALL
            });
        let page: TablePage<User> = vec![User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        }]
        .into();
        let html = policy_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("Ada"),
            "the table must still render its row, got {html}"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a table with no action prefix must not call the row policy"
        );
    }

    #[tokio::test]
    async fn action_chrome_emits_record_keys_not_display_keys() {
        // a non-PK display projection drives keyed diffs and DOM ids
        // only — edit URLs, delete dialogs, and bulk values carry the `pk`
        // projection handlers resolve as the typed PK.
        use topcoat::view::ViewExt;
        let cx = CxTestBuilder::new().build();
        let key_table = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string().to_uppercase())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_delete("/admin/users".to_string())
            .with_edit("/admin/users".to_string())
            .with_bulk_delete(true);
        let rows = vec![User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        }];
        let lower = rows[0].id.to_string();
        let upper = lower.to_uppercase();
        let page: TablePage<User> = rows.into();
        let html = key_table
            .render(&cx, page)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // URLs and bulk values: canonical PK text.
        assert!(
            html.contains(&format!("href=\"/admin/users/{lower}/edit\"")),
            "edit URL must carry the record key in {html}"
        );
        assert!(
            html.contains(&format!("value=\"{lower}\"")),
            "bulk value must carry the record key in {html}"
        );
        assert!(
            html.contains(&format!("?delete={lower}")),
            "delete dialog link must carry the record key in {html}"
        );
        assert!(
            !html.contains(&format!("value=\"{upper}\"")),
            "display key must never be a bulk value in {html}"
        );
        // Display key still drives the DOM identity.
        assert!(
            html.contains(&upper),
            "display key must still render (DOM/keyed diff) in {html}"
        );
    }

    #[tokio::test]
    async fn action_chrome_without_pk_fails_loud() {
        // chrome without `pk` would emit display keys the handlers
        // 404 on — a render error, like a missing row key, not a silent 404.
        let cx = CxTestBuilder::new().build();
        let pkless = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_bulk_delete(true)
            .with_delete("/admin/users".to_string());
        let page: TablePage<User> = vec![User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
        }]
        .into();
        let err = match pkless.render(&cx, page).await {
            Ok(_) => panic!("chrome without pk must fail loud"),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("Table::pk"),
            "the error must name the missing declaration, got {err}"
        );
    }

    #[tokio::test]
    async fn filter_widgets_render_typed_controls() {
        let cx = CxTestBuilder::new().build();
        let table_task1 = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .pk(|t| t.id.to_string())
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
        // fallback.
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
            .pk(|d| d.id.to_string())
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

    #[tokio::test]
    async fn empty_with_filters_shows_filtered_message() {
        let cx = CxTestBuilder::new().build();
        let table_task2 = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .pk(|t| t.id.to_string())
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
        // The empty-state link rebuilds the URL from the full state, clearing
        // only the dimension it names: `group_by` survives, and with a search
        // and filters active the "Clear search" link leaves the filters alone.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .pk(|t| t.id.to_string())
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

    #[tokio::test]
    async fn unknown_filter_renders_alert_banner_and_keeps_200() {
        // the list keeps a 200 but warns instead of lying about
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
            .pk(|u| u.id.to_string())
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
        // `?group_by=` must name the declared group — any other
        // value renders no headers and vanishes from pager links instead of
        // silently grouping by the single declared key.
        let cx = CxTestBuilder::new().build();
        let grouped = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
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
    async fn group_by_orders_each_row_under_its_own_header() {
        // the page-local shim must actually group. The seed is
        // deliberately interleaved in query order (draft, published, draft,
        // published), so a legend-only shim — every header, then an ungrouped
        // table — cannot satisfy the ordering assertions below.
        let cx = CxTestBuilder::new().build();
        let grouped = Table::<Task>::r#for(&cx)
            .id(|t| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t| {
                t.title.clone()
            }))
            .group_by("status", |t| t.status.clone());
        let state = TableState {
            group_by: Some("status".to_string()),
            ..TableState::default()
        };
        let task = |title: &str, status: &str| Task {
            id: uuid::Uuid::new_v4(),
            title: title.to_string(),
            status: status.to_string(),
            featured: false,
            created_at: jiff::Timestamp::now(),
        };
        let page = TablePage::from(vec![
            task("alpha", "draft"),
            task("bravo", "published"),
            task("charlie", "draft"),
            task("delta", "published"),
        ]);
        let html = grouped
            .render_with_state(&cx, page, &state, "/admin/tasks")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let at = |needle: &str| {
            html.find(needle)
                .unwrap_or_else(|| panic!("missing {needle:?} in {html}"))
        };
        // Rows are ordered by the group key (draft before published) and each
        // header sits immediately above its own rows — the stable sort keeps
        // the query's order inside a group.
        let draft_header = at("draft (2 on this page)");
        let alpha = at("alpha");
        let charlie = at("charlie");
        let published_header = at("published (2 on this page)");
        let bravo = at("bravo");
        let delta = at("delta");
        assert!(
            draft_header < alpha && alpha < charlie,
            "both draft rows must sit under the draft header, got {html}"
        );
        assert!(
            charlie < published_header,
            "the published header must follow the draft group, got {html}"
        );
        assert!(
            published_header < bravo && bravo < delta,
            "both published rows must sit under the published header, got {html}"
        );
        // The injected header carries an id derived from its group label, not
        // from its position, so the in-place morph can follow it:
        // the same contract the row ids have.
        for label in ["draft", "published"] {
            let expected = format!("id=\"{}\"", group_header_dom_id(label));
            assert!(
                html.contains(&expected),
                "the {label} header needs the stable id {expected:?}, got {html}"
            );
        }
    }

    /// a table render builds the row-action URLs from one shared base
    /// — the encoded filter transport — before the row loop, so every row's
    /// dialog opener is that base plus its own `delete=` key.
    ///
    /// The base cannot be observed as a count: `filters_param` is a pure
    /// function of the state, so a per-row rebuild produces identical bytes.
    /// This pins the shape instead — every opener shares byte-identical bytes
    /// before `delete=`, independent of the page size.
    #[tokio::test]
    async fn table_render_reuses_one_filter_transport_base_across_rows() {
        let cx = CxTestBuilder::new().build();
        let state = filters_state(&[("status", "published"), ("featured", "true")]);
        let tbl = Table::<User>::r#for(&cx)
            .id(|u: &User| u.id.to_string())
            // Every row renders a delete-dialog link, so the table needs the
            // record key those URLs carry.
            .pk(|u: &User| u.id.to_string())
            .with_delete("/admin/users".to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u: &User| {
                u.name.clone()
            }));
        let rows = |n: usize| -> Vec<User> {
            (0..n)
                .map(|i| User {
                    id: uuid::Uuid::from_u128(i as u128),
                    name: format!("user-{i}"),
                })
                .collect()
        };
        let render = async |page: TablePage<User>| {
            tbl.render_with_state(&cx, page, &state, "/admin/users")
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(&cx)
        };
        // Each row's opener, keyed off the one attribute only an action link
        // carries, with the per-row `delete` key stripped: what is left is the
        // page's shared base.
        fn delete_bases(html: &str) -> Vec<&str> {
            html.split("href=\"")
                .skip(1)
                .filter_map(|chunk| chunk.split('"').next())
                .filter(|href| href.contains("delete="))
                .map(|href| href.split("delete=").next().unwrap())
                .collect()
        }

        let one_html = render(TablePage::from(rows(1))).await;
        let eight_html = render(TablePage::from(rows(8))).await;
        let one_row = delete_bases(&one_html);
        let eight_rows = delete_bases(&eight_html);
        assert_eq!(one_row.len(), 1, "one row, one dialog opener");
        assert_eq!(eight_rows.len(), 8, "eight rows, eight dialog openers");
        // The sorted, query-encoded transport every row's link must carry.
        let transport = "filters=featured%3Atrue%2Cstatus%3Apublished";
        for base in one_row.iter().chain(eight_rows.iter()) {
            assert!(
                base.contains(transport),
                "every opener must carry the page's filter transport, got {base}"
            );
            assert_eq!(
                *base, one_row[0],
                "rows must reuse the page's one encoded base, not rebuild it per row"
            );
        }
    }

    #[tokio::test]
    async fn void_window_links_back_to_first_page() {
        // a cursor past the last row (rows deleted under pagination)
        // must offer navigation, never a pager-less dead end.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
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
    async fn skeleton_shares_the_table_root_with_the_swapped_body() {
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        let html = tbl
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
        // The skeleton and the swapped table must declare the same layout, or
        // the swap re-measures the columns: comparing the two
        // opening tags states that without pinning a class literal.
        let skeleton_table = table_tag(&html).to_string();
        // The swap payload is the table itself, under the same boundary region.
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let html = tbl
            .render_with_state(&cx, rows.into(), &TableState::default(), "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-table-root") && html.contains("data-boundary=\"table\""),
            "the swapped table must land in the skeleton's region, got {html}"
        );
        assert!(
            html.contains("Ada"),
            "swap payload must be rows, got {html}"
        );
        // Attribute order is a serializer detail: `table` merges its own
        // classes with the caller's `attrs`, so the skeleton may emit
        // `style` before `class` while the swapped table emits them the
        // other way round. What matters for GH #240 is the same layout —
        // the same classes and the same floor — not the same byte order.
        let swapped_tag = table_tag(&html);
        assert!(
            swapped_tag.contains("table-fixed") && skeleton_table.contains("table-fixed"),
            "the swapped table must declare the skeleton's layout (GH #240), got {html}"
        );
        assert_eq!(
            normalized_table_tag(swapped_tag),
            normalized_table_tag(&skeleton_table),
            "the swapped table must declare the skeleton's layout (GH #240), got {html}"
        );
    }

    #[tokio::test]
    async fn a_static_table_renders_no_runtime_bindings_at_all() {
        // a table without `live_search` has no shard to re-run, so a
        // mutation replaces its region with the response's. That is only sound
        // because the region is inert: no binding, no handler, nothing the
        // replacement could leave dead. The refresh control's absence is the
        // page's own answer to "can this table refresh in place?".
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_delete("/admin/users".to_string())
            .with_bulk_delete(true);
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let html = tbl
            .render_with_state(&cx, rows.into(), &TableState::default(), "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("data-table-revision"),
            "a static table must carry no refresh control, got {html}"
        );
        assert!(
            !html.contains("data-topcoat-"),
            "a static table's region must be inert markup, got {html}"
        );
    }

    #[tokio::test]
    async fn skeleton_carries_the_action_column_for_view_only_chrome() {
        // The skeleton's action column must count every row link `render_inner`
        // renders, `with_view` included, or the swap changes the table width.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()))
            .with_view("/admin/users".to_string());
        let skeleton = tbl
            .render_skeleton(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        let rows = vec![User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        }];
        let rendered = tbl
            .render_with_state(&cx, rows.into(), &TableState::default(), "/admin/users")
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert_eq!(
            rendered.matches(">Actions</th>").count(),
            1,
            "the real table renders one action column, got {rendered}"
        );
        assert_eq!(
            skeleton.matches(">Actions</th>").count(),
            rendered.matches(">Actions</th>").count(),
            "the skeleton must match the swapped table's column count, got {skeleton}"
        );
    }

    #[tokio::test]
    async fn rendered_rows_carry_stable_dom_ids() {
        // every rendered row exposes its morph id; re-rendering the
        // same page yields the same ids.
        let cx = CxTestBuilder::new().build();
        let tbl = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
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
}
