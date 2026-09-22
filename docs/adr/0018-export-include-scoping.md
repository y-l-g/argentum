# Export query scoping: columns declare their includes, the resource narrows

Date: 2026-09-22 — Status: accepted — Amended: 2026-09-22

## Decision

**1. The declaration lives on the column.** `TextColumn::needs(["author"])` names the relations the
projection closure reads, in whatever vocabulary the resource uses for them. It is a declaration, not a
mechanism: names are `&'static str`, opaque to the framework, meaningful only to the resource that maps
them onto typed `include(..)` calls (a type-erased column cannot name an `Include<Post, Author>`). A
column that reads no relation declares nothing, and `.needs(..)` accumulates across calls.

**2. The table gathers, the resource narrows.** `Table::include_needs()` unions its columns'
declarations; the export hands that set to `Resource::export_query(cx, needs)`. The resource is the only
layer that knows the typed includes, so it owns the mapping — and the framework owns the tenant half,
wrapping the result exactly as it wraps `query`, so an export cannot be unscoped (GH #223, ADR-0002).

**3. The default inherits `query`, unchanged.** A resource that overrides nothing exports exactly as
before. Erring towards over-fetching costs a join; dropping an include a rendered column reads breaks
the render, so the safe default is the one that keeps the old behavior, and narrowing is opt-in per
resource.

**4. Only the export narrows.** The list page keeps inheriting `query`: the export is a different reader
with a different render — it writes every column once, walks the query in cursor chunks, and has no live
re-render — so it is where the constant factor is visible.

**5. The unloaded-relation guard is the check, not a new one.** A column that reads a relation it did
not declare renders against an unloaded `Deferred`, and its `is_unloaded` guard (ADR-0011:
`"(unloaded)"` plus a `debug_assert!`) fails loudly in test builds. A resource that narrows must keep
what its `can_view` reads, since the export's visibility scan runs before any cell is written.

## Consequences

- The showcase's posts and comments tables declare their relation columns (`author`, `comments`,
  `post`), so their exports are byte-identical to before; both were already needed by a rendered column.
  The payoff accrues to a resource whose `query` carries an include its table does not render, and
  `export_query_narrows_to_the_declared_column_includes` pins that narrowing with a real relation.
- **No in-tree resource is in that position yet**, so the motivating example ("the CSV needs only
  title/status") is not reproduced: `csv_row` writes every declared column. An export column subset
  stays out of scope — this decides which includes the rendered columns need, not which columns are
  rendered.
- The name vocabulary is a string seam between two halves in the same crate tree: an unknown name is not
  an error, it just never matches, and a missing name is caught at render by the column's guard, not at
  compile time. Widening to a typed declaration would mean moving `Include` construction into the
  column, which cannot be done without naming `M`'s relation types there.
- The declaration sits on the column because the closure that reads a relation is the thing that
  declares it, so it travels with the projection when a column is moved or copied, and
  `Table::include_needs` is the mechanical union. A table-level list would be equally checkable.
- Rows-per-chunk memory is unchanged (GH #172); what changes is the per-row join work the database does
  for an include nothing renders.
