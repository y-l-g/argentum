# Confirmed mutations: the response is a page, the table is re-run by its shard

Date: 2026-09-23 — Status: accepted

## Decision

**A confirmed delete stays a POST that 303s to the list.** The handlers, their policy and
confirmation checks, their flash notification, and the no-JS path are unchanged
(`panel/actions.rs`, ADR-0004, ADR-0010). What changes is who follows the redirect: a form marked
`data-mutation-submit` — the row-delete confirm and the bulk confirm — is posted by
`crates/argentum-ui/assets/mutation-submit.js` with `fetch`, and the response is applied in place.
Without JavaScript the marker is inert and the same form POSTs and 303s.

**The mutation response is the whole list page, and the client reads exactly two things out of it.**

- **The toast**, because following the redirect consumed the one-time flash cookie that carries it
  (ADR-0010). The response's `[data-sonner-toast]` surfaces are inserted into the shell's toaster.
  They are inserted, never morphed: a morph re-syncs `data-mounted` from the response (`false`) on a
  surface `notifications.js` has already mounted, and its arming observer only sees *added* nodes,
  so the toast would stay hidden.
- **The table**, which it does **not** take from the response on a live table.

**The table is refreshed by the shard that already owns it.** A live table renders a refresh
control inside its region — a hidden `[data-table-revision]` input, declared and read by
`Table::render_inner` when the render is a live one. Writing it changes a signal the `table_search`
shard tracks, so the runtime re-fetches the shard and morphs the region: fresh rows for the *current*
query, markup hydrated by the runtime, and any rerun still in flight cancelled by the runtime's
request controller. The signal is declared inside the shard's own output, so it belongs to the
shard's content scope: its id derives from the shard invocation's identity and its call site, which
makes it stable across reruns and distinct per shard, and the runtime keeps its value when the
declaration renders again. The token is opaque; the client bumps a monotonic counter, because a
same-value signal write schedules nothing.

**A static table's region is replaced wholesale**, because it is inert markup: with no signals there
is no binding, no handler, and no shard (pinned by
`a_static_table_renders_no_runtime_bindings_at_all`). The control's presence is therefore the page's
own answer to "can this table refresh in place?", and the client needs no other test.

**The client never morphs the response into the live document.** Two properties make that path
unsound, and both are load-bearing:

- **The response renders the bare list URL, the page keeps its live state.** A delete 303s to
  `list_url(cx, slug)` — prefix and slug, no query (`panel/actions.rs`). A live table's query state
  lives in its signals and in the page-owned toolbar, both *outside* `[data-boundary="table"]`, and
  the client can neither read them (only `q`, `filters` and `bulk` have DOM transports; `sort`,
  `dir`, `cursor` and `group_by` are registry-only) nor reset them. Morphing the region would leave
  a table showing page 1 unfiltered under a toolbar still reading `?q=ada`, and the next shard rerun
  would snap the table back to the query the toolbar no longer shows.
- **Nothing the client inserts is hydrated.** The runtime hydrates only what its own units render
  (`topcoat-runtime`'s `hydrate` is not reachable from a plain script, and the runtime exposes no
  morph API), so a hand-written morph would keep matched elements and silently downgrade every node
  it inserts — sort links, pager links, the bulk transport — to their href fallback. A later shard
  rerun would keep those nodes (its morph pairs by tag), so the degradation would outlive the
  mutation.

**Failure paths stay the browser's.** A POST the server answers itself (4xx/5xx) wrote nothing — the
handlers verify CSRF and confirmation and check policy before opening the write, and any failure
rolls the transaction back — so the client hands the form back with `form.submit()` and the browser
shows the same response a no-JS POST would, at the same URL. A request that never completes leaves
the outcome unknown, so the page reloads instead of guessing.

**What the swap preserves.** The URL keeps the state the table still holds; only the dialog's own
`delete`/`open` parameters are dropped (a static table mirrors the response URL, whose state its
region now shows). The bulk selection is pruned by exactly the keys the write removed — the batch a
bulk form carried, or the record a row action names — and written back through the transport, so a
row delete leaves the rest of the selection standing while a batch clears it. Focus moves to the row
that took the deleted row's place (its own Delete control first), else the bulk trigger; a modal
dialog is closed by the client *before* the rerun, because a dialog the response's markup closes by
dropping `open` strands the document inert.

## Consequences

`Table::render_inner`'s live branch declares one extra signal per shard and renders one hidden input;
the shard's wire arguments and `TableSignals` are unchanged, and a mutation carries no markup. The
`table_search` shard re-runs on a write it did not have before, so a mutation costs one shard request
in addition to the POST it always made. `mutation-submit.js` joins the shell assets (ADR-0014) with
`data-mutation-submit`, `data-table-revision`, `data-boundary` and `data-sonner-toaster` as its hook
contract, and its pure decisions — which region a response hands over, which keys a write removed,
which submits it answers — are covered by `node --test`. The panel's own resource tables are live
(`Table::live_search`), so the in-place path is the shard path; the replacement path is what a
`live_search(false)` table gets, and no shipped showcase page exercises it end to end.

A page that renders a live table without a shard around it (`Table::render_live_with_state`, GH #154
§2) carries the same control, but its dependency attaches to the page unit instead: writing it
re-runs the page, not a shard.
