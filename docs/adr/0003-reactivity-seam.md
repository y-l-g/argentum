# Reactivity behind a boundary seam, migrating with Topcoat

Date: 2026-08-19 — Status: accepted — Supersedes: none

Argentum commits to today's Topcoat runtime (`signal q = String::new();`, `$(...)`, `#[shard]` POST endpoints) for Phase 1, hidden behind an owned seam: every Table is a Boundary and all page state (query/sort/page) is owned by the page; *how* a change triggers a server render is an internal detail of ArgentumTable. When Signals v2 + streaming (`signal(|| ...)`, `X-Topcoat-State`, `boundary` diff, `defer`) land on main, the seam migrates the whole toolkit without rewriting resources. Resources never hand-roll `#[shard]`; shard endpoints stay an optimization, not the API surface.
