# Relations: no new Relation trait, includes declared where they are read

Date: 2026-08-31 — Status: accepted — Amended: 2026-09-10, 2026-09-15, 2026-09-18, 2026-09-22

## Decision

Relations ride the resource query seam; there is no top-level `Relation` trait. `Table` and `Schema`
never write their own `#[shard]` or relation query, and `via` many-to-many stays SQL-only and out of
scope for v1 tables (use the join model's query when it is needed).

**Loader.** A page that needs a relation binds an explicit `include` on the resource's query, in two
typed steps — `let inc: toasty::stmt::Include<Post, Author> = Post::fields().author().into();`
(chaining `.into()` does not infer) — so a list of relation cells is one round trip (`NestedMerge` +
correlated subquery, no N+1). A cell that reads an unloaded `Deferred` renders `"(unloaded)"` with a
`debug_assert!` instead of silently reading data (GH #101).

**Table.** Relation cells are
`TextColumn::computed("Author", |p: &Post| p.author.get().map(|a| a.name.clone()).unwrap_or_default())`:
typed, and `searchable`/`sortable` only on local columns, since a computed column declares no predicate.

**Schema.** `Select::r#for(Post::fields().author_id()).relationship(..)` is a thin helper over the
same seam, not a second query vocabulary. It takes a **typed primary-key projection**
(`Fn(&R::Model) -> R::Model::PrimaryKey`) whose `Display` string becomes the `<option value>`; a
wrong projection fails to compile where the type differs from the PK, and the related PK must be a
single primitive implementing `Display` (composite-key and `Bytes`-key models cannot declare
relationship selects — use static options). An edit form must hydrate the FK with the same canonical
string the projection produces, or the stored value renders unselected.

**Policy.** Option loads respect the related resource's policy (GH #108): `can_view_any` denies the
whole load — no options and not the stored value, and the field surfaces `{label} is not available`
(on GET too) — while `can_view` filters loaded rows before any label renders, so a filtered-out value
is reported as invalid. The option cap counts the raw bounded fetch, before that filtering (GH #91).

**Tenancy.** The option loaders are generic over `schema::OptionSource`, whose `scoped_query` is a
required method; `option_query` runs `R::scoped_query(cx)`, so every related load carries the
framework's tenant predicate and a related resource whose tenancy cannot be scoped is a `Misdeclared`
option error rather than an unscoped fetch (GH #208, ADR-0002). `Select::relationship` still takes
the resource's `query` fn for type inference only; the loader does not call it directly.

**Option search (GH #150).** Above the cap the failure splits into `Overflow` (distinct from a driver
`LoadFailed`): a searchable select degrades to type-to-search, a non-searchable one keeps the retry
error. Search reuses the related `Table`'s declared `searchable()` columns through `search_expr(q)` —
no option-specific hook; zero searchable columns means the hard-cap fallback. The endpoint is
`GET {parent_list_url}/options?field=&q=`, `field` allow-listed to a declared searchable relationship
`Select` in the parent's form (400 otherwise), `q` trimmed and clamped to the shared query bound,
bounded at `limit(201)`, `can_view` before labels, `Denied` → 403, driver failure → 500, filtered
overflow → 200 with a keep-typing hint option, and never a whole-table load. Validation for an
overflowed searchable select is a targeted `pk_eq_expr` + `R::query` + `can_view` check (viewable →
pass, hidden/not-found → `invalid`, denied → `not available`, DB failure → retry); bounded sets keep
membership validation. The UI is a native `<select>` plus `selects.js` (debounced 200 ms, aborts
in-flight requests, preserves selection and placeholder); without JavaScript the plain select keeps
working.

**Row identity.** `Table::id` is display-only (keyed diffs, DOM ids) and `Table::pk` feeds
edit/delete URLs and bulk checkbox values, resolved by handlers as the model's typed PK. Rendering
action or bulk chrome without `pk` is a render error, not a silent 404, and single and bulk deletes
require `can_view` + `can_delete` (GH #168).
