# External gaps — what Argentum works around

This file tracks **missing or unstable APIs in upstream crates** (Toasty, Topcoat) that force Argentum to reach into internals or duplicate logic. Each gap lists: what Argentum does today, what a clean upstream API would look like, and how to migrate when it lands. The goal is to make the workarounds visible and cheap to remove.

> **Policy:** Do not hesitate to use `toasty_core` / `topcoat_core` internals inside `argentum-core` / `argentum-ui` when the public API is missing. Every such use **must** be documented here as a gap with "Where / Today / Why fragile / Clean upstream API / Argentum plan" so it can be upstreamed — you (maintainer) contribute the fix to Toasty/Topcoat and retire the entry. Keep this file focused: only gaps that truly belong upstream. Internal choices (e.g. Tailwind `@source` for `argentum-ui`, now ADR-0006) do **not** belong here.
>
> Audience: contributors; not user-facing. Link to this file from `crates/argentum-core/src/schema.rs` and `crates/argentum-core/src/resource.rs` where the workarounds live. Keep it short — one entry per gap.

---

## Toasty — building `OrderByExpr` / naming core `stmt::Path` for a field lens

**Where:** `crates/argentum-core/src/schema.rs` — the bridge helpers `lens_field_name_and_label` (field names/labels) and `pk_field_value`/`pk_eq_expr`/`pk_in_expr` (typed primary-key predicates from URL string ids, consumed by the panel's edit/delete/bulk-delete loaders); the crate's only `toasty_core` import sites.

**Today (accurate as of 2026-09-04):** Most of what this entry used to claim is **public already**: `toasty::schema` re-exports the whole app-schema surface (`crates/toasty/src/schema.rs:49` → `toasty_core::schema::{app, db, diff, mapping}`), so `M::schema()`, `Model::fields()`, `Field.name` (`FieldName::app_unwrap()`), `Field.primary_key`, and `ModelRoot::primary_key_fields()` are all reachable via the `toasty` facade without depending on `toasty-core`. PK *order-bys* no longer need `toasty_core` either: `Model::path_field::<Value>(index)` + `Path::asc()/desc()` build `OrderByExpr` through the facade (`Table::pk_order_bys`, `resource.rs`). Field-name resolution (`lens_field_name_and_label`) walks `core_path.projection.as_slice()[0]` → `M::schema().fields()[idx].name` — the `Projection` type is even re-exported as `toasty::stmt::Projection`.

What still genuinely requires `toasty_core`:
1. **Naming the conversion target.** `From<toasty::stmt::Path<M, T>> for toasty_core::stmt::Path` is public (`crates/toasty/src/stmt/path.rs`), but the core `stmt::Path` type itself is not re-exported through the facade, so holding the converted value needs the `toasty_core` path.
2. **Dynamic-value predicates.** The facade's `find_by_primary_key(Expr<M::PrimaryKey>)` is typed — generic code holding a parsed `stmt::Value` cannot build `pk == value` through it (`IntoExpr<Value>` is not implemented; `Path::eq` requires the concrete Rust type). The `pk_*` bridge helpers construct `stmt::Expr::eq(Expr::ref_self_field(fid), value)` in core and wrap via the public `Expr::from_untyped`.

**Why fragile:** those spots only. A toasty refactor of `Path`/`Projection`/`Expr` breaks them at compile time; the rest survives.

**Clean upstream API:**
```rust
// ideal — makes the bridge helpers deletable
impl<M: Model> Path<M, T> {
    pub fn field_name(&self) -> String;         // app-level name — shipped in PR #1207 (draft)
    pub fn storage_name(&self) -> String;       // #[column] override ⊕ app name — shipped in PR #1207 (draft)
    pub fn label(&self) -> String;              // capitalize(field_name) — stays in argentum
}
impl IntoExpr<T> for stmt::Value { … }          // or: trait Model { fn parse_key(s: &str) -> Option<Self::PrimaryKey>; }
```

**Argentum debt:** keep the helpers as the only `toasty_core` import sites. Follow-up #11 enriches `FieldLens` with `is_nullable`/`is_unique`/`storage_name` — implement those from the **public** `toasty::schema::app` metadata, not via `toasty_core`. When upstream lands the APIs above, replace the helpers and delete this entry.

---

## Toasty — field metadata for Schema hydration

**Where:** `TextInput::validate` / future `Create`/`Update` hydration (`crates/argentum-core/src/schema.rs:TextInput`).

**Today:** `FieldLens` is just `Path<M,T>`; `TextInput` knows `required`/`is_email` but not `is_nullable`/`is_unique`/`storage_name`. Validation manually checks `required` and hand-rolled `is_valid_email`. Follow-up #11 notes the gap.

**Clean upstream API:** Same as lens gap — `Path::is_nullable()`, `Path::is_unique()`, `Path::storage_name()`, plus `FieldTy` so `TextInput::for(...).required()` can default from `field.nullable == false`.

**Argentum plan:** Keep `FieldLens = Path<M,T>` alias for now; don't add trait until Toasty exposes it. When it does, `FieldLens` becomes a trait `Lens<M,T>` with those accessors and `TextInput` derives defaults.

**Upstream note (PR #1207):** the metadata accessors rebuild the schema set per call (`T::register`) — fine at form-setup frequency; call once at `for_lens` and store the result, never per-row. A per-type cache would be a separate upstream PR (global-registry design question).

---

## Toasty — LIKE escaping helper

**Where:** `README.md §6` and future Table `like_with_escape`.

**Today:** Portable search uses `starts_with` (safe, parameterised). Substring search would need `like_with_escape` with manual `q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")` + `format!("%{esc}%")`.

**Clean upstream API:** `fn escape_like(pattern: &str, escape: char) -> String` or `Path::contains_escaped(q: &str)` that parameterises and escapes server-side.

**Argentum plan:** Keep `starts_with` for Phase 1; add helper only when a Filter needs substring search. Don't vendor an escape fn yet.

---

## Topcoat — error conversion

**Where:** `crates/argentum-core/src/panel.rs`, `examples/showcase/src/pages/showcase/*.rs`, `examples/showcase/src/pages/showcase/db.rs`.

**Today (clean):** `toasty::Error` implements `std::error::Error + Send + Sync` → `anyhow::Error` → `topcoat::Error` via `impl<T: Into<anyhow::Error>> From<T> for topcoat::Error`. Correct pattern is `.map_err(Into::into)?` or `?` directly.

**Previous workaround (removed 2026-08-26):** `.map_err(|e| std::io::Error::other(e.to_string()))?` stringified the error, losing `is_record_not_found`/`is_unique_violation` predicates needed for spec §9 inline field errors and proper HTTP status mapping.

**Upstream gap:** None — Topcoat already supports `From`. Gap was Argentum-side misuse, now fixed. Keep `map_err(Into::into)` as the canonical pattern and avoid reintroducing stringification.

---

## Topcoat — Panel prefix vs NavigationItem

**Where:** `crates/argentum-core/src/panel.rs:Panel::nav_item` and `crates/argentum-core/src/resource.rs:NavigationItem::from_resource_with_prefix`.

**Today (clean):** `NavigationItem::from_resource_with_prefix::<R>(prefix)` and `Panel::nav_item::<R>(&self)` respect `Panel::prefix()`. `from_resource` remains as `"/admin"` shorthand for Phase-1 single-panel compat.

**Previous gap:** `NavigationItem::from_resource` hard-coded `url: "/admin"`, breaking `Panel::new("backoffice")`. No upstream gap — fix was internal.

---

## Toasty — unique-violation error predicate

**Where:** `crates/argentum-core/src/panel.rs` `check_unique` — the app-side unique check over every `unique()`-marked `TextInput`, run by `resource_create_post`/`resource_edit_post` (was previously hard-coded to `email` with a dead query behind it, GH #75 residue; generalized 2026-09-04).

**Today:** toasty exposes no unique-violation error kind. `toasty-core/src/error/` has `is_record_not_found`, `is_condition_failed`, … but no `is_unique_violation`; `#[unique]` only creates the DB index and duplicates surface as an unclassified driver error. The app layer is therefore the only duplicate guard: `check_unique` queries the field path via `TextInput::eq_filter` (public facade) and maps a hit to `"<Label> has already been taken"`. Driver-level violations that slip past the check (concurrent writes) propagate as errors — string-matching `"unique"`/`"duplicate"` was removed per this entry's rule.

**Why fragile:** the app-side check races with concurrent inserts (TOCTOU); only a driver-level predicate closes it.

**Clean upstream API:** `Error::is_unique_violation()` (or `ErrorKind::UniqueViolation { constraint }`) surfaced by the SQL drivers.

**Argentum plan:** keep `check_unique` as the UX layer; when the predicate lands, map it to the constrained field's inline error and delete this entry. Never string-match driver error messages.

---

## Topcoat — memoize(as_ref) error conversion

**Where:** `examples/showcase/src/pages/showcase/db.rs` `users()` — the only sanctioned stringification site.

**Today:** `#[memoize(as_ref)]` caches `Result<&T, &Error>`; `topcoat::Error` wraps `anyhow::Error`, which is not `Clone`, so a memoized fallible loader cannot hand back an owned `topcoat::Error`. The workaround converts via `std::io::Error::other(e.to_string())`, which erases typed predicates (`is_record_not_found`, …) at the memo boundary.

**Why fragile:** the pattern is easy to cargo-cult into non-memoized call sites (it was, once — removed 2026-08-26 per the Topcoat error-conversion entry above).

**Clean upstream API:** memoize supporting non-`Clone` error types — e.g. cache `Result<Arc<T>, Arc<Error>>` and return `Result<&T, &Arc<Error>>`, or an `Arc`-backed `Error` clone.

**Argentum plan:** keep the workaround localized to `db.rs` `users()`; non-memoized loaders use `.map_err(Into::into)`. Retire when memoize supports `Arc`'d errors.

---

## Topcoat — interpolating an opaque `impl View` (`NodeClassify` blanket impl)

**Where:** every Argentum helper boundary — the page handlers in `crates/argentum-core/src/panel.rs` (`resource_list` etc., all return `BoxView<'_>`) and interpolation sites like `suspense(fallback: skeleton, (lazy_rows.boxed()))` (`panel.rs:662`); same shape in `argentum-ui` composites.

**Today (accurate as of 2026-09-04):** `(expr)` interpolation requires `NodeClassify`, implemented only for five hard-coded types (`Child`, `BoxView`, `ScopeView`, `MoveView`, `LiveView`) via `impl<T: NodeViewParts> NodeClassify for T` (`topcoat-view/src/internal/node_classify.rs:24`, a `pub mod internal` file). A helper returning `Result<impl View>` yields an opaque that implements `View` but not `NodeClassify`, so it cannot be interpolated — every helper must return `BoxView` or be boxed at the call site. The error names the wrong trait (`NodeViewParts not implemented for impl View`) and suggests nothing.

**Why fragile:** `internal` has no stability guarantee, and classification sits on Argentum's hottest path (every helper boundary). An upstream rename of `NodeClassify`/`NodeViewParts` breaks all of it at once.

**Clean upstream API:** `impl<T: View> NodeClassify for T` (or classification without a trait), or failing that a diagnostic saying "box it with `.boxed()`".

**Argentum plan:** keep `BoxView` returns and `.boxed()` at interpolation sites — they are cheap anyway. Drop the boxes and this entry when the blanket impl lands. Found as the first consumer of tokio-rs/topcoat#373.

---

## Topcoat — hand-registered fallible async pages (`ThenView` is internal)

**Where:** the single `use topcoat::view::internal::ThenView;` in `crates/argentum-core/src/panel.rs:10`; `Box::pin(ThenView::new(async move { .. }))` in `resource_list`/`resource_create`/`resource_create_post`/`resource_edit`/`resource_edit_post`/`resource_delete`/`resource_bulk_delete` (`panel.rs:620-1180`) and the nested `lazy_rows` suspense child (`panel.rs:643`).

**Today:** `Panel` hand-registers pages through the public registry seam — `pages: Vec<PageFn>` built with `PageFn::new(method, path, handler)` (`panel.rs:164+`). `PageRenderFn` is sync (`fn(&Cx, Body) -> BoxView`), so a fallible async page body (auth check → `Err(forbidden().into())`, awaits, `Ok(view! { .. })`) can only be expressed by adapting the future with the internal `ThenView` and boxing — the same adaptation `#[page]` performs internally.

**Why fragile:** `topcoat::view::internal` is explicitly unstable; a refactor there moves Argentum's central page adapter. There is no public alternative: nothing else converts a fallible future-of-a-view into a view.

**Clean upstream API:** a public constructor on the page side — e.g. `page_from_fn(.., |cx, body| async { Result<impl View> })` — or `pub use ThenView` (a future-of-a-view becomes a view; its `Err` becomes the view's error).

**Argentum plan:** keep the one internal import and the `ThenView`-boxed shape (it also powers the streamed `lazy_rows`); swap to the public constructor and delete this entry when it lands.

---

## Topcoat — no idiomatic list-of-views

**Where:** row rendering in `crates/argentum-core/src/resource.rs:1073-1080` (`for (key, cells) in &row_data { table_row(key: key_for_row, for cell in cells { table_cell((cell.clone())) }) }`), the skeleton loop `resource.rs:1190` (`key: i`), the empty state `resource.rs:1454` (`key: "empty"`).

**Today:** dynamic lists render as `for` loops inside one `view!`/component call, with `key:` on every component call. `BoxView` is the only container and is documented only as the fix for multiple `return` sites; `Child` has no `FromIterator`; nothing documents whether `Vec<BoxView>` can be interpolated.

**Why fragile:** the loop pattern works and nothing breaks — but the rule is discoverable only by trial, and a future refactor toward pre-built view lists (`Vec<BoxView>`) has no documented support either way.

**Clean upstream API (any one):** `impl FromIterator<BoxView<'a>> for Child<'a>`, a `fragment!` helper, or a `view.md` note stating the intended pattern is a `for` loop inside one `view!` with `key:` on component calls.

**Argentum plan:** keep loop + `key:`; adopt whichever upstream shape lands and delete this entry.

---

## Topcoat — views may borrow only `cx` (undocumented borrow rule)

**Where:** every view-building site; representatives: `crates/argentum-core/src/panel.rs:637,652` (`prefix.clone()`, `title.clone()` captured into the async page body) and `crates/argentum-core/src/resource.rs:1079` (`(cell.clone())` per cell).

**Today:** views borrow the `Cx` they were built against, therefore a view can never borrow anything owned by the code that returns it. Capturing a handler local (`TablePage`, `Vec<NavigationItem>`, form maps) fails with E0515 `cannot return value referencing local variable`; Argentum clones at capture sites. The rule is permanent (it follows from the lazy model) but documented nowhere — each new page rediscovers it by fighting E0515.

**Why fragile:** a docs gap, not an API one — nothing breaks, but every page pays a defensive clone. The upstream fix is a paragraph, not code.

**Clean upstream API:** one paragraph in `view.md` — "templates may borrow `cx` freely; anything else they capture must be owned or borrowed from `cx`" — plus a worked example.

**Argentum plan:** keep the clones (cheap: `String`s and small vecs); delete this entry when the paragraph lands.

---

## Toasty — instance → field-value extraction (row keys, cells)

**Where:** `crates/argentum-core/src/resource.rs` — `Table::id` row-key closure and `TextColumn`'s projection closure.

**Today (accurate as of 2026-08-30):** Toasty models are plain structs; the `Model`/`Field` traits expose paths and schema metadata but **no way to read a field value off an instance generically** (`Load` only goes `Value → model`). Argentum therefore requires the projection as a user-written closure:
- `Table::id(|u| u.id.to_string())` — the row key for `key:`-ed rows (row identity is mandatory; render errors without it).
- `TextColumn::for_lens(lens, |u| u.name.clone())` — the cell projection.

Typos in either closure fail at compile time, so the old stringly-typed `HasId`/`GetField` dispatch (GH #10, panic-on-unknown) is gone — but every table/column repeats the field read.

**Why fragile:** nothing breaks (closures are typed); the cost is ergonomic repetition, and a silent mismatch between the lens (query side) and the closure (render side) cannot be detected — e.g. a column whose lens says `email` but whose closure reads `name` still compiles.

**Clean upstream API:**
```rust
trait Model {
    fn primary_key(&self) -> Self::PrimaryKey;        // row keys without the closure
    // and/or: generated per-field accessors usable as `Fn(&Self) -> &T`
}
```

**Argentum plan:** keep the closures until upstream exposes instance→value access; then default `Table::id` / `TextColumn` projections from the lens and delete this entry. #10 shipped the closures in 7689381; this entry tracks the upstream API that would retire them.

---

## Phase 2 — FileUpload / Repeater / Tenancy / Relations — no new upstream gap

**Where:** `crates/argentum-core/src/schema.rs` (`FileUpload`, `Repeater`), `crates/argentum-core/src/tenancy.rs` (`Tenant`, `tenant_id`), `examples/showcase/src/app.rs` (`PostResource::query` `include(author)` + `include(comments)` + `TextColumn::computed` / `Select::relationship`).

**Today (accurate as of 2026-08-31):** Phase 2 ships without new upstream APIs. `FileUpload` stores a `String` path (`image_path`) and `Repeater` is a nested `Schema` (`tags`) — both are local form state, no Toasty asset/file handling needed. Tenancy is `Cx::with(Tenant(id))` + `tenant_id(cx)` reading `request_context::<Tenant>` → `Parts.extensions::<Tenant>` → `x-tenant-id` header (for `Router::handle` tests), and `Resource::query(cx)` filters `tenant_id().eq(tid)` — no Tower layer. Relations are `Deferred<Author>` + `include(Post::fields().author())` (one round-trip, `NestedMerge`, no N+1) + `TextColumn::computed` and `Select::for(...).relationship(AuthorResource::query, |a| a.name.clone())` reusing `Resource::query` so tenancy is preserved (ADR-0011). The `author include` gap is **retired**: it was never an upstream gap, just the use of existing `include` + `computed`/`relationship` on the `Resource::query` seam. `Table::group_by` + `to_csv` are in-memory (`BTreeMap` count) and CSV is via `GET /admin/{slug}/export` (`text/csv` + `Content-Disposition`), not requiring `GROUP BY` SQL or `Sse` streaming.

**Why not fragile:** no `toasty_core` import beyond the two `schema.rs` bridge helpers already documented above; no new Topcoat API.

**Clean upstream API:** none needed. If Toasty exposes `GROUP BY`/`SUM` or file-asset handling, `Table::group_by` and `FileUpload` can delegate without changing `Resource`s; until then the in-memory shims stay.

**Argentum plan:** keep as-is; do not add entries for `FileUpload`/`Repeater`/`Tenant`/`author include`. If a future phase needs SQL `GROUP BY` or binary file handling, document the new gap then.

---

## How to retire entries

1. Add the upstream API (or feature-flag it).
2. Grep for `toasty_core::` — should become zero outside `crates/argentum-core/src/schema.rs` bridge helpers.
3. Update the bridge helpers to delegate to the new public API, keep signature.
4. Delete the entry here and reference the Toasty/Topcoat PR that closed it.

Last updated: 2026-09-04 (PK tie-breaker gap retired, GH #76: toasty's `normalize_cursor_order` appends the physical PK columns to ambiguous cursor orderings internally since tokio-rs/toasty#1142, so the app-level suffix in `Table::order_bys`/`order_bys_for_state` was dropped; the no-sortable paginated fallback builds PK order via the public `Model::path_field` + `Path::asc` facade — no `toasty_core` remaining in `resource.rs`). 2026-09-04 (four topcoat view-layer gaps documented from the tokio-rs/topcoat#373 consumer review — interpolation of opaque `impl View`, internal `ThenView` for hand-registered fallible async pages, list-of-views shape, undocumented `cx`-only borrow rule; `TOPCOAT_PR_373_FEEDBACK.md` retired into this file). 2026-08-31 (Phase 2: FileUpload/Repeater/Tenancy/Relations need no upstream gap, author include gap retired; grouping/export are in-memory shims). 2026-08-30 (instance→field-value extraction gap documented with the typed Table projection, GH #10). 2026-08-29: memoize error conversion + missing unique-violation predicate documented; showcase stringification corrected. 2026-08-28: Tailwind `@source` moved to ADR-0006 (internal), policy added: use internals freely and document missing public APIs here.
