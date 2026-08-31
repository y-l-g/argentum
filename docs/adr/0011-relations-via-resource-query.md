# Relations via Resource::query + include, not a new Relation seam

Date: 2026-08-31 — Status: accepted — Supersedes: none

## Context

Phase 2 needs `HasMany`/`BelongsTo` in tables and forms: a `Post` list should show `author.name`, a `Post` form should offer a `Select` for `author_id` that loads `Author` options. Toasty already has `Deferred<T>` + `include` preloading (`User::all().include(User::fields().posts()).exec(&mut db).await?` → `post.author.get()`), and `Resource::query(cx) -> Query<List<M>>` is the single seam for tenancy/soft-delete (ADR-0002). Filament does not introduce a top-level `Relation` type for this; its `Select::make('author_id')->relationship('author', 'name')` and `TextColumn::make('author.name')` are thin wrappers over the existing `Schema`/`Table` seams that call `relationship` to resolve options via the related `Resource::query`.

We considered two seams:

- **(A) Filament-faithful, no new seam:** Keep `Resource::query` as the single row-scoping seam. `Table` relation columns are `TextColumn::computed` that read `Deferred` after an explicit `include` in the loader (`Post::all().include(Post::fields().author()).exec(&mut db).await?` → `post.author.get().name`), and `Schema` `Select::for(Post::fields().author_id()).relationship(AuthorResource::query)` loads options via the related `Resource::query`. No new `Relation` trait; `Table::columns` and `Schema::new` stay.
- **(B) New Relation seam:** Introduce `trait Relation { fn related_query(cx) -> Query; fn foreign_key() -> Path; ... }` (`HasMany`/`BelongsTo`) and make `Table`/`Schema` depend on it. More explicit, but adds a new top-level seam that duplicates `Resource::query` tenancy logic and will be hard to reverse. It also forces `via` many-to-many (SQL-only, out-of-scope for v1 tables per `README.md:363`) into the same trait.

## Decision

We pick **(A)** — no new `Relation` trait. Relations stay as:

- **Loader:** `Resource::query(cx)` + explicit `include` (one round-trip, `NestedMerge` + correlated subquery, no N+1). Every `Table`/`Form` that needs a relation does `query.include(Post::fields().author())` before `exec`. `via` many-to-many remains SQL-only and out-of-scope for v1 tables; use the join model query when DynamoDB compatibility is desired.
- **Table:** `TextColumn::computed("Author", |p: &Post| p.author.get().map(|a| a.name.clone()).unwrap_or_default())` (typed, `searchable`/`sortable` only on local columns; computed columns declare no predicate). `Table::id(|p| p.id.to_string())` stays the single row-key.
- **Schema:** `Select::for(Post::fields().author_id()).relationship(AuthorResource::query, |a: &Author| a.name.clone())` — a thin helper that takes a `FieldLens<M, T>` and a `Resource` to resolve options via `Resource::query(cx)`. It does not introduce a new query seam; it reuses `Resource::query` so tenancy is preserved.

This is clean (one seam, no `Macroable`/`statePath`), fast (explicit `include` preloading, no per-row `exec`, deterministic cursors with PK tie-breaker), and flexible (works with any `Resource`, any `Deferred`, and any `Policy`).

## Consequences

- `Panel` and `Resource::query` stay the single owners of tenancy; `Table`/`Schema` never hand-roll `#[shard]` or `Relation::query`. The `ArgentumTable` `Boundary` + `#[memoize]` seam from Phase 1 remains unchanged; relations are just `include` + `computed`/`Select`.
- `cargo test --workspace` proves relations via `Router::handle` (list shows `author.name`, form `Select` shows `Author` options, tenancy via `Resource::query` yields 404 for wrong tenant) and via `CxTestBuilder` (column/field rendering). No `#[shard]` in `Resource` impls.
- `via` many-to-many (`has_many(via = memberships.group)`) stays SQL-only and out-of-scope for v1 tables; it is documented as future work, not a Phase 2 ticket.
- `CONTEXT.md` does not gain a new top-level `Relation` term; `Resource` and `Table` glossary entries are clarified to mention `include` + `computed`/`Select::relationship`.
