# Tenancy via Cx, in-memory grouping and CSV export

Date: 2026-08-31 — Status: accepted — Supersedes: none

## Context

Phase 2 needs per-tenant scoping and table polish (grouping, export) without new storage or streaming seams. `Resource::query(cx)` is already the single seam for tenancy (ADR-0002) via `Cx::with`. Toasty has no `GROUP BY`/`SUM` yet (`roadmap #421/#425`) and `Sse` streaming for export is not required for v1 CSV. Filament's tenancy is `Tenant` scoped value + global scope filter; Argentum must match without a `withoutGlobalScope` footgun. Grouping must not require SQL until Toasty exposes it.

We considered:

- **Tenancy:** Tower layer vs `Cx::with(Tenant(id))` + `tenant_id(cx)` helper reading `Tenant` → `Parts.extensions` → `x-tenant-id` header (for `Router::handle` tests). Layer would couple HTTP middleware to domain; `Cx` keeps business logic in `Resource::query` so every loader (`Table`/`Form`/`Action`) is scoped by filtering `tenant_id().eq(tid)`. No `withoutGlobalScope`.
- **Grouping:** SQL `GROUP BY` via raw `toasty::sql::query` behind `trait Aggregate` vs in-memory `Table::group_by(|row| key)` + `BTreeMap` count/sum over `page.rows`. SQL shim would leak raw SQL into `Table` and require Toasty `GROUP BY` before `via` many-to-many; in-memory keeps `Table` pure and matches Filament's summarizers for v1.
- **Export:** `Sse` streaming vs `GET /admin/{slug}/export` `Route` reusing `Resource::query` + `Table` filters/sort, streaming `text/csv` with `Content-Disposition: attachment`. `Sse` would require streaming boundary infra; download route is simpler, URL-shareable, and `Table::to_csv` already has the `page.rows` in memory (no pagination for CSV).

## Decision

- **Tenancy:** `Tenant(pub Uuid)` is a `Cx` scoped value (`cx.with(Tenant(id))`); `fn tenant_id(cx) -> Option<Uuid>` checks `Tenant` → `Parts.extensions` → `x-tenant-id`. Every tenant-scoped `Resource::query(cx)` does `filter(tenant_id().eq(tid))` when `Some`. `Policy::canViewAny`/`canView`/`canUpdate`/`canDelete` checks `tenant_id` as well (dedicated `from_u128(9999)` deny in showcase). Panel routes keep the check in both page and `#[procedure]`/`#[shard]`.
- **Grouping:** `Table::group_by(key: impl Fn(&M) -> String)` stores `GroupKey<M>`; `TableState` parses `?group_by=`; `Table::render` groups `page.rows` in-memory via `BTreeMap<String, usize>` and shows `format!("{key} ({count})")` headers. No SQL `GROUP BY` until Toasty exposes it; raw-SQL `Aggregate` shim stays behind `trait Aggregate` and is not used in `Table`.
- **Export:** `Table::to_csv(&TablePage)` generates RFC4180 `String` (header + rows, quotes when `,`, `"`, `\n`); `Panel` owns a per-resource `GET /admin/{slug}/export` `Page` (via `resource_export` helper, currently showcased as `#[route]` in `showcase` for HTTP `text/csv` + `Content-Disposition: attachment; filename="posts.csv"`). The loader reuses `Resource::query` + `TableState` filters/sort and `include` so `author.name` appears when the table has relation columns.

This keeps one tenancy seam, no new `Relation` trait, and no `via` many-to-many in tables (SQL-only, out-of-scope for v1).

## Consequences

- `cargo test --workspace` proves tenancy via `Router::handle` with `Cx::with(Tenant)` and `x-tenant-id` header, `Policy` 403/404, and grouping/export via `Router::handle` (`text/csv` + `Content-Disposition`). `Table` stays a `Boundary` (`data-boundary="table"`); `TableState` is parsed once for loader and render.
- `EXTERNAL_GAPS.md` records that `FileUpload`/`Repeater`/`Tenant`/`author include` need no new upstream API; grouping/export are in-memory shims. When Toasty exposes `GROUP BY`, `Table::group_by` can delegate without changing `Resource`s.
- Showcase at `/admin/posts` demonstrates tenancy (tenant 1 vs 2 rows), `SelectFilter`/`TernaryFilter`/`DateFilter`, `group_by` and export, `FileUpload`/`Repeater`, `Panel::brand`/`dark_mode`, and `benchmarks/` Phase-2 budget (50 rows, 2 includes, `<40ms p50`).
