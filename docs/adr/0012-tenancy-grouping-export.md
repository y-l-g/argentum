# Tenancy via Cx, in-memory grouping and CSV export

Date: 2026-08-31 — Status: accepted — Amended: 2026-09-10, 2026-09-15, 2026-09-22, 2026-09-24

## Decision

**Tenancy.** `Tenant(uuid::Uuid)` is a `Cx`-scoped value (`cx.with(Tenant(id))`) and `tenant_id(cx)`
reads it. A Tower layer is rejected: it would couple HTTP middleware to the domain and put the scope
somewhere other than the resource. `requires_tenant` defaults to `false`; when a resource declares
it, the **framework** applies the tenant predicate to every loader through `scoped_query`, derived
from the model's `tenant_id` unless the resource declares `tenant_scope` (ADR-0002). The
`x-tenant-id` header fallback is not part of the production path; tests and the harness inject
`Tenant` through request extensions.

**Grouping.** `Table::group_by(key: impl Fn(&M) -> String)` stores a `GroupKey<M>`; `TableState`
parses `?group_by=`; render groups the page's rows in memory through a `BTreeMap` and shows
`{key} ({count} on this page)` headers — counts are page-local and labelled as such (GH #92). A real
summarizer and the raw-SQL `trait Aggregate` shim are not part of the vocabulary (GH #107): a
page-local sum would be a misleading number for exactly the large tables aggregation exists for, so
a sum waits for upstream `GROUP BY` (#118), at which point `Table::group_by` can delegate without
changing resources.

**Export.** `Table::csv_header` and `Table::csv_row` generate RFC4180 fragments (header + rows,
quoting when a value holds `,`, `"` or a newline). `Panel` owns a per-resource
`GET {prefix}/{slug}/export` route that serves
`text/csv` with a sanitized `Content-Disposition: attachment; filename="..."`. The loader walks the
filtered, sorted query in cursor chunks (GH #172), so a 10k-row export holds one chunk plus one CSV
fragment, and the cap counts **viewable** rows: visibility is applied before the cap (GH #145). A
full 10,001-row scan window with rows left beyond it refuses with the same 413 (GH #279), so the
export never returns a partial file. Its
includes come from `Resource::export_query(cx, needs)`, whose default is `query(cx)` unchanged and
which an override may narrow to the relations the exported columns declared (`TextColumn::needs`,
`Table::include_needs`, ADR-0018); the tenant scope is the framework's, applied to `export_query`
exactly as to `query`.

## Consequences

The showcase's posts list demonstrates tenancy (tenant 1 vs 2 rows),
`SelectFilter`/`TernaryFilter`/`DateFilter`, grouping, export, `FileUpload`/`Repeater` and
`Panel::brand`. It paints light by default: the stored `theme` preference wins in both directions, the
header toggle is the only thing that turns dark on, and `dark_mode(true)` remains available for a
dark-first panel. The `benchmarks/` Phase-2 `<40ms p50` figure is a target, not a gate — the harness
prints it for reference only (GH #171) and `benchmarks/results/` is gitignored. Grouping and export
stay in-memory shims until Toasty exposes `GROUP BY` (#118).
