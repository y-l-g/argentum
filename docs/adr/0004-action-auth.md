# Action authorization is transactional and per-record

Date: 2026-08-19 — Status: accepted — Supersedes: none

Every procedure-backed Action runs inside a DB transaction: fetch the target via `Resource::query(cx)` (the tenancy seam), check `Policy` against that record inside the handler, mutate, commit. No bulk authorization shortcut in Phase 1 — bulk delete re-fetches through the query and checks each record. `shouldSkipAuthorization` exists only as an explicit, audited opt-in. Shard/procedure inputs are untrusted; the check always runs against the fetched row, never the passed ID alone.

## Amendment (2026-09-10)

`shouldSkipAuthorization` never shipped. The invariant held — every mutation runs in a framework-owned transaction and checks `can_*` against the loaded record (GH #84/#86) — but there is no `Action` value type or `#[procedure]`; mutations are `Resource` record fns receiving `&mut dyn toasty::Executor`. The standalone `Policy<R>` trait exists with no callers (wire-or-remove decision pending).

## Amendment (2026-09-15)

The wire-or-remove decision is resolved: **removed** (GH #109). The standalone `Policy<R>` trait had zero callers — the enforced seam is `Resource::can_view_any/can_view/can_create/can_update/can_delete`, default-deny, checked in page and POST handlers. Keeping a parallel authorization vocabulary next to the live one was the confusion the ADR flagged; the module, its re-exports, its tests, and the README mention are gone. The `Policy` *term* in CONTEXT.md still names the `can_*` rules.
