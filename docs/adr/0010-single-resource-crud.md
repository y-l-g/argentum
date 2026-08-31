# Single-resource CRUD — Create/Edit, Action, Policy, Notification

Date: 2026-08-31 — Status: accepted — Supersedes: none

## Context

Phase 1 required a usable admin at `/admin/users` with search/sort/paginate/create/edit/delete, all policy-checked, no N+1, and a notification that survives Table swaps. The prior slice proved list-only: `Panel` owned the `Router` and `Shell`, `Resource` mapped one `Model` to a `Table`/`Schema` stub, and `TableState` drove search/sort/pagination via `Resource::query`. Missing were the writable seams: `Schema` hydration/dehydration, `Action` via `#[procedure]` in a transaction, `Policy` default-deny, `Notification` in the `Shell` `Boundary`, and `Table` as a `Boundary` with `#[memoize]` dedup.

## Decision

We make the `User` `Resource` fully writable on the existing seams:

- **Panel** owns list/create/edit/delete/bulk-delete routes (`Panel::resource::<R>()` materializes `GET /admin/{slug}`, `GET /admin/{slug}/create`, `POST /admin/{slug}/create`, `GET /admin/{slug}/{id}/edit`, `POST /admin/{slug}/{id}/edit`, `POST /admin/{slug}/{id}/delete`, `POST /admin/{slug}/bulk-delete`) and the `Shell`'s notification `Boundary` (`fixed top-4 right-4`, `border-border bg-background shadow-sm`). `Router::builder().discover().cookies()` installs the cookie layer; `Panel::layout_shell` renders the complete document.
- **Schema** hydrates/dehydrates via typed lenses: `TextInput::for(User::fields().name()).required().email().unique()` fails to compile on bad field; `Schema::hydrate` fills `value` attrs from the `Model` (via `Resource::hydrate_form_values`), `Schema::validate` collects `required`/`email` inline per-field and `Schema::render_with` shows them in the reserved `text-sm text-destructive` slot. Unique is checked app-side until Toasty exposes `is_unique_violation`; DB unique errors are mapped to the field.
- **Action** runs `#[procedure]`-compatible handlers inside a transaction, re-fetching the target via `Resource::query` (the tenancy seam) and checking `Policy` (`viewAny`/`view`/`create`/`update`/`delete`, default-deny, page+procedure) against the fetched row. `Delete` `requires_confirmation` renders a `Dialog`/`confirm` page; `BulkDelete` re-fetches each `id` via `Resource::query` and checks `Policy::delete` per row, all-or-nothing.
- **Notification** is a transient `status+title` (`success`/`error`, ~4s) produced by `Action`/`Procedure` results, stored as `Set-Cookie` (`argentum_notification`) and `?notification=` query fallback, rendered in the `Shell`'s top-level `Boundary` so it survives `Table` `Boundary` swaps.
- **Table** is a `Boundary` by default (`Table::boundary(true)`, `defer(true)` shows `skeleton` rows). Its `search`/`sort`/`page` state is `TableState` parsed once from `?q=`/`?sort=`/`?dir=`/`?after=`/`?before=`; `Table::order_bys_for_state` adds the PK tie-breaker for deterministic cursors. A `#[shard]` (`table_shard`) with `#[memoize]`-deduped loader (`memoized_dummy`) demonstrates the reactivity seam: `signal q = String::new()` + `$(q.get())` + `#[shard]` swaps only the `Boundary`, while `defer`+`boundary` diff remains the future migration path. `Table::with_delete`/`with_bulk_delete` add row `Delete` buttons and bulk bar.

`Resource` now exposes `can_view_any`/`can_view`/`can_create`/`can_update`/`can_delete` (default-deny) and `create_record`/`update_record`/`delete_record`/`bulk_delete_records`/`hydrate_form_values` (default error, showcase `UserResource` overrides with `toasty::create!`/`update!`/`delete` via `Resource::query`). `Table::key_for` exposes the typed row-key closure for `key:`-ed rows and for `find_by_id` via `Resource::query` + in-memory `key_for` match (until Toasty exposes PK `Eq`).

Considered: (A) Tower middleware for auth (rejected: `Cx`-scoped `fn require_*` per README §2), (B) `via` many-to-many in tables (deferred to Phase 2, SQL-only), (C) `GROUP BY` aggregates (raw-SQL shim only).

## Consequences

- `cargo run` at `/admin/users` now supports search/sort/paginate/create/edit/delete/bulk-delete, all policy-checked, no N+1, benchable. `cargo test --workspace` proves list/create/edit/delete/bulk, `viewAny`/`view` gating, `Boundary`/`defer`/`memoize` dedup, and notification survival.
- `Panel` remains the single owner of `Router`/`Db`/`Shell`; `Resource::query` stays the single tenancy seam; `Schema` stays the single form seam; `Table` stays the single list seam. No `Resource` hand-rolls `#[shard]`; the seam migrates from `#[shard]` to `defer`+`boundary` without rewriting resources.
- `cargo test --workspace` / `clippy -D warnings` / `fmt` stay green per commit; `examples/showcase/tests/{admin,create_check,edit_check,delete_check,bulk_check}.rs` cover the vertical slice.
