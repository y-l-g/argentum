# Relations via Resource::query + include, not a new Relation seam

Date: 2026-08-31 — Status: accepted — Supersedes: none

## Context

Phase 2 needs `HasMany`/`BelongsTo` in tables and forms: a `Post` list should show `author.name`, a `Post` form should offer a `Select` for `author_id` that loads `Author` options. Toasty already has `Deferred<T>` + `include` preloading (`User::all().include(User::fields().posts()).exec(&mut db).await?` → `post.author.get()`), and `Resource::query(cx) -> Query<List<M>>` is the single seam for tenancy/soft-delete (ADR-0002). Filament does not introduce a top-level `Relation` type for this; its `Select::make('author_id')->relationship('author', 'name')` and `TextColumn::make('author.name')` are thin wrappers over the existing `Schema`/`Table` seams that call `relationship` to resolve options via the related `Resource::query`.

We considered two seams:

- **(A) Filament-faithful, no new seam:** Keep `Resource::query` as the single row-scoping seam. `Table` relation columns are `TextColumn::computed` that read `Deferred` after an explicit `include` in the loader (`Post::all().include(Post::fields().author()).exec(&mut db).await?` → `post.author.get().name`), and `Schema` `Select::for(Post::fields().author_id()).relationship(AuthorResource::query)` loads options via the related `Resource::query`. No new `Relation` trait; `Table::columns` and `Schema::new` stay.
- **(B) New Relation seam:** Introduce `trait Relation { fn related_query(cx) -> Query; fn foreign_key() -> Path; ... }` (`HasMany`/`BelongsTo`) and make `Table`/`Schema` depend on it. More explicit, but adds a new top-level seam that duplicates `Resource::query` tenancy logic and will be hard to reverse. It also forces `via` many-to-many (SQL-only, out-of-scope for v1 tables per README §10) into the same trait.

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

## Amendment (2026-09-10)

Loader details the decision left open: the `include` must be bound in two typed steps (`let inc: toasty::stmt::Include<Post, Author> = Post::fields().author().into()`) — chaining `.into()` does not infer. Unloaded relation cells render `"(unloaded)"` with a `debug_assert!` instead of silently reading data (GH #101). `Select::relationship` option values come from the related `Table::id` display key and the loader does not consult the related `Resource`'s `can_view_any` — both tracked separately (see #91); both resolved by the 2026-09-15 amendments below.

## Amendment (2026-09-15)

Relationship option identity (GH #108): `Select::relationship` takes a **typed primary-key projection** (`Fn(&R::Model) -> R::Model::PrimaryKey`) whose `Display` string becomes the `<option value>` — this supersedes the two-argument call shown in the Decision section. The related table's `Table::id` row-key projection is no longer consulted for option values; it stays the list's row identity for DOM ids, bulk values and edit/delete URLs. A wrong value projection fails to compile where the projected type differs from the PK. The related PK must be a single primitive implementing `Display` (composite-key and `Bytes`-key models cannot declare relationship selects — use static options). Edit forms must hydrate the FK with the same canonical string the projection produces, or the stored value renders unselected.

Option loads respect the related resource's policy (GH #108), resolving the policy caveat in the 2026-09-10 amendment: `can_view_any` denies the whole load — the select renders no options and not the stored value, the field surfaces `{label} is not available` (on GET too), and an untouched denied value on an optional select submits empty; a submit that still carries a value fails closed. `can_view` filters loaded rows before any label renders, so a filtered-out value is absent and reported as invalid. The option cap counts the raw bounded fetch, before that filtering (GH #91): counting viewable rows only would let one hidden record defeat the cap and silently truncate a larger table.

## Amendment (2026-09-18, GH #168)

Display key vs record key. The sentence above ("it stays the list's row
identity for DOM ids, bulk values and edit/delete URLs") is superseded:
`Table::id` is display-only (keyed diffs, DOM ids) and a new `Table::pk`
projection feeds edit/delete URLs and bulk checkbox values, resolved by
handlers as the model's typed PK. Rendering action or bulk chrome without
`pk` is a render error, not a silent 404; single and bulk deletes require
`can_view` + `can_delete` (the edit contract).

## Amendment (2026-09-18, GH #150)

Server-side option search for tables above the cap. The cap failure splits into `Overflow` (distinct from driver `LoadFailed`): searchable `Select::relationship(..).searchable()` degrades to type-to-search, non-searchable keeps the retry error.

- **Search contract:** reuse the related `Table`'s declared `searchable()` columns via `search_expr(q)` ("option search searches the related resource's declared searchable columns"). No option-specific hook. Zero searchable columns → hard-cap fallback (unfiltered bounded load, `Overflow` on large tables).
- **Endpoint:** `GET {parent_list_url}/options?field=&q=` (e.g. `/admin/posts/options?field=author_id&q=ada`), `field` allow-listed to a declared searchable relationship `Select` in the parent `R::form(cx)` (400 otherwise). `q` trimmed + clamped to the shared query bound; empty `q` is the bounded head. Bounded `limit(201)`, `can_view` before labels, `Denied` → 403, driver failure → 500, filtered overflow → 200 with a keep-typing hint option. No whole-table loads.
- **Validation:** overflowed searchable selects use a targeted check (`pk_eq_expr` + `R::query` + `can_view`): viewable → pass, hidden/not-found → `invalid`, denied → `not available`, DB failure → retry. Bounded sets keep membership validation.
- **UI:** native `<select>` + `selects.js` fetch (debounced 200ms, abort in-flight, selection + placeholder preserved). Initial over-cap render keeps the stored value + search input + hint; no-JS keeps the plain select (documented limitation). No custom combobox (tracked separately if needed).
