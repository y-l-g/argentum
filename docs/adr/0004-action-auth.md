# Action authorization is transactional and per-record

Date: 2026-08-19 — Status: accepted — Supersedes: none

Every procedure-backed Action runs inside a DB transaction: fetch the target via `Resource::query(cx)` (the tenancy seam), check `Policy` against that record inside the handler, mutate, commit. No bulk authorization shortcut in Phase 1 — bulk delete re-fetches through the query and checks each record. `shouldSkipAuthorization` exists only as an explicit, audited opt-in. Shard/procedure inputs are untrusted; the check always runs against the fetched row, never the passed ID alone.

## Amendment (2026-09-10)

`shouldSkipAuthorization` never shipped. The invariant held — every mutation runs in a framework-owned transaction and checks `can_*` against the loaded record (GH #84/#86) — but there is no `Action` value type or `#[procedure]`; mutations are `Resource` record fns receiving `&mut dyn toasty::Executor`. The standalone `Policy<R>` trait exists with no callers (wire-or-remove decision pending).
