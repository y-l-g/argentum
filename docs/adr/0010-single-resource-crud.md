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

## Amendment (2026-09-10)

Implementation drifted from this record; the code won. There is no `#[procedure]`/`Action` value — mutations are `Resource` record fns called by handlers inside a framework-owned transaction (#84), with policy re-checked on the loaded snapshot (#86). `table_shard`/`memoized_dummy` were deleted (#74); the live shard is slug-dispatched `table_search` behind `Table::live_search` (#104), and the table is a `data-boundary="table"` wrapper, not a `Boundary` type. The PK tie-breaker moved into Toasty (#76). App-side unique checks do not map DB violations to fields: a concurrent write surfaces as a 500 (#88, open).

## Amendment (2026-09-15)

The notification flash cookie is `__Host-argentum_notification` with `Secure` (GH #149), matching the session and CSRF cookies' hardened contract — removals carry the same attributes so browsers honor the clear.

## Amendment (2026-09-15, Topcoat bump — GH #120/#124/#126/#139)

Topcoat now flushes pending `Set-Cookie`s on error responses (topcoat#408), so the notification is cookie-only and one-time: the `?notification=` query fallback is gone, and mutations answer `303 See Other` (`see_other`, topcoat#398) with the flash cookie on the redirect — following it consumes the toast, so reloads never replay it (GH #126). The flash value is Topcoat's `CookieStore` JSON under `__Host-argentum_notification` (GH #139) instead of the hand-rolled `status:title` codec. Row identity in `view!` loops is now a loop-level `#[key(...)]` (topcoat#410) — the old per-component `key:` prop is removed; tables key rows from the row key, never the loop index (GH #124). `topcoat::Error` is Arc-backed and `Clone` (topcoat#396), so the memoized loader hands out the typed error without stringification (GH #120).

## Amendment (2026-09-21, unique implies presence — GH #189)

`TextInput::unique()` now implies `required`. The decision above left the empty case open, and the framework stores `""` rather than NULL (GH #89), so an empty value on an optional `unique()` field was *a value*: the first empty submit passed the app-side check and wrote `""`, and the second met the constraint instead of the form rule — 500 when the record fn stored what it was handed, or a misleading "has already been taken" when it trimmed first (the probe trims, the stored value did not). Either way the panel enforced one rule and the database another. Recording the true rule at declaration time is one validation rule with no query: an empty `unique()` field reports `"<Label> is required"` inline, `.optional()` does not lift it (in either call order, and whether the marker came from the builder or from `#[unique]` via the lens), and the app-side probe never sees an empty value. The required marker in the rendered control reads the same predicate, so a unique field cannot be refused for emptiness while displaying as optional. Both halves are now declaration-checked: `Panel::build` refuses a `.unique()` marker on a column with no unique index (single-field or composite — the same `lens_field_unique` reachability the marker itself uses), so the panel can no longer enforce a rule the database does not. Treating `""` as absent was the alternative, rejected because it would still disagree with the index and would cost an extra probe per empty submit. Measured in the same change: SQLite's default `BINARY` collation is case-sensitive for both the index and the probe, so the two agree there as well (`crates/argentum-core/tests/sqlite.rs`).
