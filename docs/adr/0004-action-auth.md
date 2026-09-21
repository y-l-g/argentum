# Action authorization is transactional and per-record

Date: 2026-08-19 — Status: accepted — Supersedes: none

Every procedure-backed Action runs inside a DB transaction: fetch the target via `Resource::query(cx)` (the tenancy seam), check `Policy` against that record inside the handler, mutate, commit. No bulk authorization shortcut in Phase 1 — bulk delete re-fetches through the query and checks each record. `shouldSkipAuthorization` exists only as an explicit, audited opt-in. Shard/procedure inputs are untrusted; the check always runs against the fetched row, never the passed ID alone.

## Amendment (2026-09-10)

`shouldSkipAuthorization` never shipped. The invariant held — every mutation runs in a framework-owned transaction and checks `can_*` against the loaded record (GH #84/#86) — but there is no `Action` value type or `#[procedure]`; mutations are `Resource` record fns receiving `&mut dyn toasty::Executor`. The standalone `Policy<R>` trait exists with no callers (wire-or-remove decision pending).

## Amendment (2026-09-15)

The wire-or-remove decision is resolved: **removed** (GH #109). The standalone `Policy<R>` trait had zero callers — the enforced seam is `Resource::can_view_any/can_view/can_create/can_update/can_delete`, default-deny, checked in page and POST handlers and on relationship option loads (GH #108). Keeping a parallel authorization vocabulary next to the live one was the confusion the ADR flagged; the module, its re-exports, its tests, and the README mention are gone. The `Policy` *term* in CONTEXT.md still names the `can_*` rules.

## Amendment (2026-09-22, GH #112)

The post-commit half of the mutation vocabulary now exists: `Resource::after_commit(cx, Committed<Self::Model>)`, default no-op, called by all four write handlers **after `tx.commit()` and before the response**. `Committed` carries `Mutation::Create/Update/Delete` and the rows the mutation wrote (a bulk delete is one value, not one per row). Before this, email/webhooks/audit had no correct home: inside a record fn the effect leaks on rollback, and a second `Db` handle while the transaction holds the pool is the discipline the handlers exist to enforce. The hook runs with the transaction gone, so it may open its own handle.

Three properties are load-bearing, and each is pinned by a test through a real `Panel`: it runs **once** per committed write; it never runs when nothing committed (validation, policy, record-fn error, commit error); and a hook that fails is **logged and ignored** — the write is committed, so an error page would misreport it. Retries and delivery guarantees are explicitly not promised; an outbox written in the hook is the app's shape, not the framework's.

Two signatures changed so the seam can name what it wrote: `create_record` and `update_record` return `Result<Self::Model>` instead of `Result<()>`. Neither value is inventable by the framework — a generated key is knowable only from the row the write produced (`toasty::create!` hands it back), and the committed state of an update only from the row the write reloaded (Toasty's instance update applies the database's returned values to the model, so `Ok(rec)` after `toasty::update!` is authoritative). Passing the pre-write snapshot instead would have a watcher notify its subscribers with stale values. That is what puts `Clone` on `Resource::Model`: the handler keeps the rows it loaded while the record fn consumes them, and a bulk delete hands over up to `MAX_BULK_IDS` of them (the one copy the framework still makes — dropping it would mean taking the loaded rows by reference in two more record-fn signatures, which is not worth it for a bounded copy on an admin action).

The hook is an **associated fn**, not a method: every `Resource` hook is (`create_record` and friends take no `self`), so there is no instance to receive. An app that needs configuration in its hook gets it where its record fns already do — a module-level value, or the app context through `cx`.

**`Action` stays a term, not a type.** The first suggestion in #112 — "a small `Action` value type for non-CRUD mutations with before/after hooks" — is not built: the four record fns are the CRUD implementation, `Mutation` names their kinds, and a non-CRUD operation (publish, archive) is still a record fn or a hand-written page. Whether that deserves a first-class value with its own hooks is a separate question with no caller yet.
