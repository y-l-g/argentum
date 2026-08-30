# Action authorization is transactional and per-record

Date: 2026-08-19 — Status: accepted — Supersedes: none

Every procedure-backed Action runs inside a DB transaction: fetch the target via `Resource::query(cx)` (the tenancy seam), check `Policy` against that record inside the handler, mutate, commit. No bulk authorization shortcut in Phase 1 — bulk delete re-fetches through the query and checks each record. `shouldSkipAuthorization` exists only as an explicit, audited opt-in. Shard/procedure inputs are untrusted; the check always runs against the fetched row, never the passed ID alone.
