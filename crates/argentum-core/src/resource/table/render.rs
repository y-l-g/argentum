//! [`Table`] HTML rendering: `render`/`render_with_state`/`render_skeleton` plus the chrome.
//!
//! Moved verbatim from `resource.rs` (GH #133): no behavior change.

use argentum_ui::{
    ButtonSize, ButtonVariant, alert_dialog, button, button_variants, dialog_content,
    dialog_description, dialog_footer, dialog_header, dialog_title, icons, input as ui_input,
    pagination, pagination_content, pagination_item, pagination_next, pagination_previous, table,
    table_body, table_cell, table_head, table_header, table_row,
};
use topcoat::context::Cx;
use topcoat::icon::icon;
use topcoat::runtime::Event;
use topcoat::{Result, view::*};

use super::super::filter::Filter;
use super::super::state::{TablePage, TableSignals, TableState, encode_path_segment, row_dom_id};
use super::Table;

/// Keystroke-quiet delay before a live search input reloads the grid
/// (GH #172, ~150-250ms): `assets/live-search.js` waits this long after the
/// last keystroke, then forwards the value through the bound transport below,
/// so typing "published" triggers one reload instead of nine. The forwarded
/// write is an ordinary signal write, so Topcoat's abort-in-flight
/// coalescing still applies to the resulting rerun.
pub(crate) const LIVE_SEARCH_DEBOUNCE_MS: u32 = 200;

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
    /// Composes the synced `argentum-ui` primitives and Token classes
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
        let edit_prefix = self.edit_prefix.clone();
        let with_actions = delete_prefix.is_some() || edit_prefix.is_some();
        let with_bulk = self.bulk_enabled();
        // Record keys feed URLs and bulk values, which handlers resolve as
        // the typed PK (GH #168): chrome without `pk` would emit display keys
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
        let bulk_bar_view: BoxView<'_> = if with_bulk {
            let bulk_action = format!("{}/bulk-delete", self.delete_prefix.clone().unwrap());
            let csrf = crate::csrf::current_token(cx);
            // No visible `ids` field (GH #151): the transport is fed by the row
            // checkboxes (`bulk.js`), and the destructive submit ships disabled
            // so an empty submit cannot be produced from the UI. On a live
            // table the selection lives in a signal instead (GH #166): the
            // transport is bound to it and the submit's disabled state derives
            // from it, so a shard rerun re-renders both from the selection
            // rather than dropping it.
            let (transport_attrs, submit_attrs) = match &signals {
                Some(signals) => {
                    let bulk = signals.bulk.clone();
                    (
                        attributes! {
                            cx =>
                            type="hidden"
                            name="ids"
                            :value=$(bulk.get())
                            @change=$(|e: Event| bulk.set(e.target.value))
                            data-bulk-ids=""
                        },
                        attributes! {
                            cx =>
                            type="submit"
                            :disabled=$(bulk.get().is_empty())
                            data-bulk-submit=""
                        },
                    )
                }
                None => (
                    attributes! { cx => type="hidden" name="ids" value="" data-bulk-ids="" },
                    attributes! { cx => type="submit" disabled="" data-bulk-submit="" },
                ),
            };
            view! {
                cx =>
                <form
                    method="post"
                    action=(bulk_action)
                    class="flex gap-2 p-3 border-b border-border"
                    data-bulk-form=""
                >
                    <input type="hidden" name="csrf_token" value=(csrf)>
                    <input (transport_attrs)>
                    button(
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Md,
                        attrs: submit_attrs,
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
                // alongside valid ones). Conversely an invalid-only request
                // applies nothing, so "other filter(s)" would be the lie
                // (GH #170) — key off applied predicates, not raw entries.
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
        };
        // Precompute the row presentation so template bodies capture only
        // owned data — the lazy view outlives this call, so it must never
        // borrow `self` or `page`.
        //
        // The per-row delete URL (GH #151) opens the confirmation dialog on
        // the list page (`?delete=<key>`) instead of posting straight away.
        // The per-row edit URL (GH #162, Filament's `recordActions`
        // `EditAction`) links straight to `{prefix}/{key}/edit`.
        //
        // Both URLs — and the bulk checkbox values below — carry the *record*
        // key (GH #168), resolved by handlers as the model's typed PK. The
        // display `key` stays on keyed diffs and DOM ids.
        // `record_id` can only be empty on chromeless tables (guarded above),
        // which render no URLs and no bulk column to read it.
        let row_data: Vec<RowView> = page
            .rows
            .iter()
            .map(|row| {
                let key = row_key(row);
                let record_id = record_key.as_ref().map(|f| f(row)).unwrap_or_default();
                let cells: Vec<String> = self
                    .columns
                    .iter()
                    .map(|col| col.render_cell(row))
                    .collect();
                let edit_url = edit_prefix
                    .as_ref()
                    .map(|prefix| format!("{}/{}/edit", prefix, encode_path_segment(&record_id)));
                let delete_url = delete_prefix
                    .is_some()
                    .then(|| state.with_delete_dialog(path, &record_id));
                RowView {
                    key,
                    record_id,
                    cells,
                    edit_url,
                    delete_url,
                }
            })
            .collect();
        // Row keys must be injective within a page (GH #96): duplicates corrupt
        // keyed diffs and bulk selection (two rows, one checkbox value).
        debug_assert!(
            {
                let mut seen = std::collections::HashSet::new();
                row_data.iter().all(|row| seen.insert(row.key.clone()))
            },
            "duplicate Table::id keys in one page: Table::id must be injective"
        );
        // The confirmation dialog lives with the delete chrome (GH #151); the
        // live-search page renders it outside the shard region instead.
        let delete_dialog = self.render_delete_dialog(cx, state, path).await?;

        // Body-only branch (GH #133): the empty and rows pages share the one
        // chrome wrapper built below — only the grid body differs. Group
        // headers and the pager exist solely on rows pages: an empty page
        // renders the honest empty cell instead (its pager would be empty
        // anyway, and grouping an empty page yields no headers).
        let mut group_views: Vec<BoxView<'_>> = Vec::new();
        let mut pager_views: Vec<BoxView<'_>> = Vec::new();
        let grid: BoxView<'_> = if page.rows.is_empty() {
            let empty_cell = self
                .render_empty_cell(cx, state, path, with_actions, with_bulk, signals.as_ref())
                .await?;
            view! {
                cx =>
                table(
                    (head)
                    (empty_cell)
                )
            }
            .boxed()
        } else {
            // Grouping (in-memory, count summarizer) — only when `?group_by=`
            // names the declared group; unknown values render nothing (GH #92).
            // Rendered after skeleton/empty so defer shows skeleton and empty shows
            // the honest empty state even when `?group_by=` is set (GH #75).
            // Counts are page-local (GH #92): label them as such so page 1 never
            // reads as a table total.
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
                            <div class="px-4 py-2 bg-muted text-sm font-medium">
                                (text)
                            </div>
                        }
                        .boxed(),
                    );
                }
            }
            pager_views = pager;
            // One grid body for grouped and ungrouped pages: `group_views` is
            // empty unless `?group_by=` named the declared group.
            view! {
                cx =>
                table(
                    (head)
                    table_body(
                        #[key(row.key.as_str())]
                        for row in &row_data {
                            let key_for_row = row.key.clone();
                            let key_for_select = row.record_id.clone();
                            let edit_for_row = row.edit_url.clone();
                            let open_for_row = row.delete_url.clone();
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
                                for cell in &row.cells {
                                    table_cell((cell.clone()))
                                }
                                if edit_for_row.is_some() || open_for_row.is_some() {
                                    table_cell(
                                        <div class="flex gap-2">
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
                                            if let Some(url) = open_for_row {
                                                <a
                                                    href=(url)
                                                    class=(button_variants(
                                                        ButtonVariant::Destructive,
                                                        ButtonSize::Md,
                                                    ))
                                                >
                                                    "Delete"
                                                </a>
                                            }
                                        </div>
                                    )
                                }
                            )
                        }
                    )
                )
            }
            .boxed()
        };

        // One chrome wrapper for both branches: search bar, filter bar, bulk
        // bar, warning, group headers, grid, pager, dialog (GH #133).
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
                (grid)
                for p in pager_views {
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
    ///
    /// Behavior asset: Escape/backdrop dismissal and the `data-dialog-close`
    /// cancel hook need `assets/dialog.js` (`argentum_ui::DIALOG_JS`, which
    /// also mirrors the dismissal into `?open=false`), emitted by
    /// `Panel::render_document` on every document with shell assets (see
    /// ADR-0014). The dialog primitives are vendored under the ADR-0007 sync
    /// guard so they carry no note themselves. Without the document scripts
    /// Cancel still navigates and Delete still POSTs.
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
                self.delete_prefix.is_some() || self.edit_prefix.is_some(),
                self.bulk_enabled(),
                None,
            )
            .await?;
        let column_count = self.columns.len();
        let with_actions = self.delete_prefix.is_some() || self.edit_prefix.is_some();
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
        Ok(if self.is_boundary {
            // The busy state rides on the morph boundary (GH #160) so assistive
            // tech sees the live region, not just the swapped root below it.
            view! { cx => <div data-boundary="table" aria-busy="true">(inner)</div> }.boxed()
        } else {
            inner.boxed()
        })
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

    /// Eager live-search input for live tables (GH #104): the signal-backed
    /// input plus the GET form as `<noscript>` fallback. Rendered eagerly
    /// above the streamed region; the shard invocation that fills the grid
    /// lives in the streamed region ([`Self::render_live_invocation`]) so the
    /// grid can only ever render once per response.
    ///
    /// The visible input is deliberately unbound (GH #172): typing stays
    /// local until it pauses for [`LIVE_SEARCH_DEBOUNCE_MS`], then
    /// `assets/live-search.js` forwards the value through the bound hidden
    /// transport, whose `@change` writes `q` and clears the cursors (a new
    /// term is a new result set). The shard re-renders in place (GH #151).
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
                    placeholder="Prefix search…"
                    aria-label="Live prefix search table"
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
    /// region (GH #104). The signal handles travel as arguments; every
    /// tracked read inside the shard becomes a `dep` marker the browser
    /// watches, so sort/filter/pager/search changes re-render the grid in
    /// place (GH #151).
    pub(crate) async fn render_live_invocation<'a>(
        &self,
        cx: &'a Cx,
        _state: &TableState,
        path: &str,
        signals: TableSignals,
    ) -> Result<BoxView<'a>> {
        use crate::panel::table_search;

        // No snapshot here (GH #157): grouping travels as the `group_by`
        // live signal (seeded from the page state by the caller) and the
        // shard normalizes on read (GH #153) — `_state` stays only so the
        // seam keeps its shape for a future grouping control.
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
    /// the signals (GH #166) — the counterpart of [`Self::render_live_search_bar`].
    ///
    /// Hoisting matters for focus: a `<select>` change writes the `filters`
    /// signal, and a bar rebuilt by that rerun would collapse the native popup
    /// and drop keyboard context. The grid renders without the bar
    /// (`Table::filters(false)`), so the control the user touched is never
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
        // Called with raw page state (GH #153): normalize so the no-JS
        // fallback form carries the same normalized values the GET path would.
        let state = self.normalize_state(state);
        self.render_filter_bar(cx, &state, path, Some(signals))
            .await
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
        // No `filters.is_empty()` early return: the caller (`render_inner`)
        // already guards on `show_filters`, so an empty bar is unreachable.
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
        let mut heads: Vec<BoxView<'_>> = Vec::with_capacity(self.columns.len());
        for col in &self.columns {
            let label = col.label().to_string();
            // A static preview renders plain labels: no link to an interaction
            // the page does not honor (GH #151).
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
                        attrs: attributes! { class=(head_class) aria-sort=(aria_sort) },
                        (header)
                    )
                }
                .boxed(),
            );
        }
        if with_actions {
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

/// Precomputed per-row presentation for the grid body: the display row key,
/// the record key, the rendered cells, and the optional Edit / delete-dialog
/// action URLs. A struct (not a tuple): five anonymous positions would
/// mislead readers and trip `clippy::type_complexity` (GH #162).
///
/// `key` is the `Table::id` display projection (keyed diffs, DOM ids);
/// `record_id` is the `Table::pk` projection (URLs, bulk values), resolved
/// by handlers as the typed PK (GH #168).
struct RowView {
    key: String,
    record_id: String,
    cells: Vec<String>,
    edit_url: Option<String>,
    delete_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{
        DateFilter, SelectFilter, Sort, TernaryFilter, TextColumn, VariantFilter,
    };
    use std::collections::HashMap;
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
        // chromeless table needs no `pk` (GH #168): nothing emits URLs.
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
    async fn action_chrome_emits_record_keys_not_display_keys() {
        // GH #168: a non-PK display projection drives keyed diffs and DOM ids
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
        // GH #168: chrome without `pk` would emit display keys the handlers
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
        // The empty-state link used to rebuild the URL from `sort` alone:
        // `group_by` was always dropped, and with a search + filters active
        // the "Clear search" link also cleared the filters.
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
        // GH #92: `?group_by=` must name the declared group — any other
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
    async fn void_window_links_back_to_first_page() {
        // GH #98: a cursor past the last row (rows deleted under pagination)
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
    async fn skeleton_shares_table_root_and_defer_clears_for_swap() {
        let cx = CxTestBuilder::new().build();
        let deferred = Table::<User>::r#for(&cx)
            .id(|u| u.id.to_string())
            .pk(|u| u.id.to_string())
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

    #[tokio::test]
    async fn rendered_rows_carry_stable_dom_ids() {
        // GH #104: every rendered row exposes its morph id; re-rendering the
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
