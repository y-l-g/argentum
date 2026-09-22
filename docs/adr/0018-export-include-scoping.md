# Export query scoping: columns declare their includes, the resource narrows

Date: 2026-09-22 — Status: accepted — Supersedes: none — Amends: ADR-0012 (the export's reuse of `Resource::query`)

## Context

The CSV export reuses `Resource::query` wholesale (ADR-0012), so it loads every
relation that query includes — including relations only another page reads.
The list's live table and the export render the same columns, but a resource's
`query` is shared by the list, the export, the detail page and the edit form's
relationship loads, so an include added for any one of them rides along on all
of them. The #172 grill deferred trimming it (decision 10: keep inheriting
`R::query`) because the contract it needs did not exist: a column's cell
projection is a closure, and Toasty exposes no instance→field reflection
(upstream #119), so the framework cannot see which relations a column touches.
The streaming half of #172 landed, leaving this constant-factor over-fetch
(GH #177).

The alternatives were: leave it (an include nothing reads costs a join per
chunk of every export), infer includes from the lens (the lens names a column,
not a relation), or declare them.

## Decision

**1. The declaration lives on the column.** `TextColumn::needs(["author"])`
names the relations the projection closure reads, in whatever vocabulary the
resource uses for them. It is a declaration, not a mechanism: names are
`&'static str`, opaque to the framework, meaningful only to the resource that
maps them onto typed `include(..)` calls (a type-erased column cannot name an
`Include<Post, Author>`). A column that reads no relation declares nothing, and
`.needs(..)` accumulates across calls.

**2. The table gathers, the resource narrows.** `Table::include_needs()` unions
its columns' declarations; the export hands that set to a new
`Resource::export_query(cx, needs)`. The resource is the only layer that knows
the tenancy filter and the typed includes, so it owns the mapping:

```rust
fn query(cx: &Cx) -> Query<List<Post>> {
    PostResource::base(cx, &IncludeNeeds::from(["author", "comments"]))
}

fn export_query(cx: &Cx, needs: &IncludeNeeds) -> Query<List<Post>> {
    PostResource::base(cx, needs)
}
```

**3. The default inherits `query`, unchanged.** A resource that overrides
nothing exports exactly as before. Erring towards over-fetching costs a join;
dropping an include a rendered column reads breaks the render, so the safe
default is the one that keeps the old behaviour. Narrowing is opt-in per
resource.

**4. Only the export narrows.** The list page keeps inheriting `query` (the
#172 grill's decision 10). The export is a different reader with a different
render — it writes every column once, walks the query in cursor chunks, and has
no live re-render — so it is where the constant factor is visible; the list's
sharper budget is about keystroke latency, not include count.

**5. The unloaded-relation contract is the guard, not a new check.** A column
that reads a relation it did not declare renders against an unloaded
`Deferred`; its `is_unloaded` guard (ADR-0011: `"(unloaded)"` plus a
`debug_assert!`) fails loudly in test builds. A resource that narrows must keep
what its `can_view` reads — the export's visibility scan runs before any cell
is written, and an un-included relation panics there — and the same guard is
what makes that loud rather than silent.

## Consequences

- The showcase's posts and comments tables declare their relation columns
  (`author`, `comments`, `post`), so their exports are byte-identical to
  before: both were already needed by a rendered column. The payoff accrues to
  a resource whose `query` carries an include its table does not render — an
  include added for `view_relations`, for instance — and the core test
  `export_query_narrows_to_the_declared_column_includes` pins that narrowing
  with a real relation (fails if the export goes back to `R::query`).
- **No in-tree resource is in that position yet**, so the motivating example in
  GH #177 ("the CSV needs only title/status") is not reproduced by this change:
  `csv_row` writes every declared column, and the showcase's posts table
  renders author and comments. Trimming further would need an export column
  subset, which is the last bullet here — this ADR delivers the include
  contract, not a column subset.
- The name vocabulary is a string seam between two halves in the same crate
  tree. An unknown name is not an error, it just never matches; a *missing*
  name is caught at render by the column's guard, not at compile time. Widening
  to a typed declaration would mean moving `Include` construction into the
  column, which cannot be done without naming `M`'s relation types there.
- The declaration was put on the column rather than on the table: the closure
  that reads a relation is the thing that declares it, so the declaration
  travels with the projection when a column is moved or copied, and
  `Table::include_needs` is the mechanical union. A table-level list would be
  equally checkable — a column could still omit a declaration under either
  shape, and the guard is what catches that — so this buys cohesion, not
  safety.
- The `#[derive(Resource)]` path is deliberately untouched: the derive cannot
  declare a `table`, so a derived resource is never mounted and never serves an
  export. An `export_query = path` key would be unreachable API.
- Rows-per-chunk memory is unchanged (that was GH #172); what changes is the
  per-row join work the database does for an include nothing renders.
- Column subsets for export (exporting fewer columns than the table renders)
  stay out of scope: this decides *which includes* the rendered columns need,
  not which columns are rendered.
