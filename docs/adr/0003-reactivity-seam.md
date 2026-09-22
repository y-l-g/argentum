# Reactivity behind a boundary seam, migrating with Topcoat

Date: 2026-08-19 — Status: accepted — Supersedes: none

Argentum commits to today's Topcoat runtime (`signal q = String::new();`, `$(...)`, `#[shard]` POST endpoints) for Phase 1, hidden behind an owned seam: every Table is a Boundary and all page state (query/sort/page) is owned by the page; *how* a change triggers a server render is an internal detail of ArgentumTable. When Signals v2 + streaming (`signal(|| ...)`, `X-Topcoat-State`, `boundary` diff, `defer`) land on main, the seam migrates the whole toolkit without rewriting resources. Resources never hand-roll `#[shard]`; shard endpoints stay an optimization, not the API surface.

## Amendment (2026-09-10)

The migration happened on Topcoat main 0.8: `suspense` streams the resource list behind `Table::render_skeleton`, reruns morph in place (#392), shards take `Signal<T>` params (#393), and the keystroke-live `table_search` shard landed behind `Table::live_search` (#104) — without rewriting resources. The `boundary(true)`/`defer(true)` demo hooks were removed by GH #220 (no demo existed; the grid is always a boundary) and return with the demo that needs them; there is no `Boundary` component type anymore, and `live!`/`emit!` were not adopted.
