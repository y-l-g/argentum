# Export query scoping: columns declare their includes, the resource narrows

Date: 2026-09-22 — Status: accepted — Amended: 2026-09-22, 2026-09-25

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
resource. Since the 2026-09-25 amendment the export default delegates to `Resource::query_with`, which
itself defaults to `query` unchanged, so this clause still holds for a resource that overrides neither.

**4. Every loader narrows through `query_with`.** The list, edit, delete, bulk-delete, unique-value,
relationship-option and pagination-probe loaders name the includes they read and the resource answers
with the matching branch of `Resource::query_with` (2026-09-25 amendment; the export was the first
reader, and only the export narrowed before it). A loader that reads no relation asks for an empty set;
the detail page reads `view_relations`, an opaque hook with no declaration, so it keeps the full
`query`.

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

## Amendment — 2026-09-25

**`query_with(cx, needs)` generalizes the export seam to every loader** (GH #298). `Resource::query`
stays the full base query; the new `query_with` takes the same `IncludeNeeds` vocabulary and defaults
to `query(cx)`, so a resource that overrides nothing is unchanged and narrowing stays opt-in.
`export_query` now defaults to `query_with`, so one override narrows the list and the export together.

The loaders that read no relation — the edit page (both loads), delete, bulk delete, the unique-value
probe, the relationship option lists and their targeted FK existence check, and the pagination
probes — pass an empty set. The list and the export pass `Table::include_needs()`. The detail page
keeps `query`, because `view_relations` is an opaque hook the framework cannot inspect for a
declaration. The export's visibility scan also passes an empty set: it renders no cell, so it needs
only what `can_view` reads, which an override states unconditionally.

Consequence the option case states: an option load renders a value and a label per row, and both
projections are opaque closures, so the option loader asks for nothing. An option label therefore
projects the related record's own columns; a label that reads a relation panics in `Deferred::get`.
