# External gaps — what Argentum works around

This file tracks **missing or unstable APIs in upstream crates** (Toasty, Topcoat) that force Argentum to reach into internals or duplicate logic. Each gap lists: what Argentum does today, what a clean upstream API would look like, and how to migrate when it lands. The goal is to make the workarounds visible and cheap to remove.

> **Policy:** Do not hesitate to use `toasty_core` internals inside `argentum-core` when the public API is missing. Every such use **must** be documented here as a gap with "Where / Today / Why fragile / Clean upstream API / Argentum plan" so it can be upstreamed — you (maintainer) contribute the fix to Toasty/Topcoat and retire the entry. Keep this file focused: only gaps that truly belong upstream. Internal choices (e.g. Tailwind `@source` for `argentum-ui`, ADR-0006) do **not** belong here.
>
> Audience: contributors; not user-facing. Keep it short — one entry per gap.

---

## Toasty — building `OrderByExpr` / naming core `stmt::Path` for a field lens

**Where:** `crates/argentum-core/src/schema.rs` — the bridge helpers `lens_field_name_and_label` (field names/labels) and `pk_field_value`/`pk_eq_expr`/`pk_in_expr` (typed primary-key predicates from URL string ids, consumed by the panel's edit/delete/bulk-delete loaders); plus `crates/argentum-core/src/cursor.rs` (cursor `Value`/`ValueRecord` handling).

**Today:** Most of what this entry used to claim is **public already**: `toasty::schema` re-exports the whole app-schema surface, so `M::schema()`, `Model::fields()`, `Field.name`, `Field.primary_key`, and `ModelRoot::primary_key_fields()` are all reachable via the `toasty` facade without depending on `toasty-core`. PK *order-bys* no longer need `toasty_core` either: `Model::path_field::<Value>(index)` + `Path::asc()/desc()` build `OrderByExpr` through the facade (`Table::pk_order_bys`). Field-name resolution (`lens_field_name_and_label`) walks `core_path.projection.as_slice()[0]` → `M::schema().fields()[idx].name` — the `Projection` type is even re-exported as `toasty::stmt::Projection`.

What still genuinely requires `toasty_core`:
1. **Naming the conversion target.** `From<toasty::stmt::Path<M, T>> for toasty_core::stmt::Path` is public, but the core `stmt::Path` type itself is not re-exported through the facade, so holding the converted value needs the `toasty_core` path.
2. **Dynamic-value predicates.** The facade's `find_by_primary_key(Expr<M::PrimaryKey>)` is typed — generic code holding a parsed `stmt::Value` cannot build `pk == value` through it (`IntoExpr<Value>` is not implemented; `Path::eq` requires the concrete Rust type). The `pk_*` bridge helpers construct `stmt::Expr::eq(Expr::ref_self_field(fid), value)` in core and wrap via the public `Expr::from_untyped`.
3. **Cursor values.** `cursor.rs` holds `toasty_core::stmt::Value` / `ValueRecord` for cursor encode/decode.

**Why fragile:** those spots only. A toasty refactor of `Path`/`Projection`/`Expr`/`Value` breaks them at compile time; the rest survives.

**Clean upstream API:**
```rust
// ideal — makes the bridge helpers deletable
impl<M: Model> Path<M, T> {
    pub fn field_name(&self) -> String;         // app-level name (proposed in draft toasty#1207, not at the locked rev)
    pub fn storage_name(&self) -> String;       // #[column] override ⊕ app name
    pub fn label(&self) -> String;              // capitalize(field_name) — stays in argentum
}
impl IntoExpr<T> for stmt::Value { … }          // or: trait Model { fn parse_key(s: &str) -> Option<Self::PrimaryKey>; }
```

**Argentum debt:** keep the helpers as the `toasty_core` import sites. Follow-up work enriching `FieldLens` with `is_nullable`/`is_unique`/`storage_name` should use the **public** `toasty::schema::app` metadata, not more `toasty_core`. When upstream lands the APIs above, replace the helpers and delete this entry.

---

## Toasty — field metadata for Schema hydration

**Where:** `TextInput::validate` / `Create`/`Update` hydration (`crates/argentum-core/src/schema.rs:TextInput`).

**Today:** `FieldLens` is just `Path<M,T>`; `TextInput` knows `required`/`is_email` but not `is_nullable`/`is_unique`/`storage_name`. Validation manually checks `required` and hand-rolled `is_valid_email`.

**Clean upstream API:** Same as lens gap — `Path::is_nullable()`, `Path::is_unique()`, `Path::storage_name()`, plus `FieldTy` so `TextInput::for(...).required()` can default from `field.nullable == false`.

**Argentum plan:** Keep `FieldLens = Path<M,T>` alias for now; don't add trait until Toasty exposes it. When it does, `FieldLens` becomes a trait `Lens<M,T>` with those accessors and `TextInput` derives defaults.

**Upstream note:** the draft metadata accessors rebuild the schema set per call — fine at form-setup frequency; call once at `for_lens` and store the result, never per-row. A per-type cache would be a separate upstream PR (global-registry design question).

---

## Toasty — LIKE escaping helper

**Where:** `README.md §6` and table search.

**Today:** Portable search uses `starts_with` (safe, parameterised). Substring search would need manual `q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")` + `format!("%{esc}%")`.

**Clean upstream API:** `fn escape_like(pattern: &str, escape: char) -> String` or `Path::contains_escaped(q: &str)` that parameterises and escapes server-side.

**Argentum plan:** Keep `starts_with`; add helper only when a filter needs substring search. Don't vendor an escape fn yet.

---

## Toasty — unique-violation error predicate

**Where:** `crates/argentum-core/src/panel.rs` `check_unique` — the app-side unique check over every `unique()`-marked `TextInput`, run by `resource_create_post`/`resource_edit_post`.

**Today:** toasty exposes no unique-violation error kind. `toasty-core/src/error/` has `is_record_not_found`, `is_condition_failed`, … but no `is_unique_violation`; `#[unique]` only creates the DB index and duplicates surface as an unclassified driver error. The app layer is therefore the only duplicate guard: `check_unique` queries the field path via `TextInput::eq_filter` (public facade) and maps a hit to `"<Label> has already been taken"`. Driver-level violations that slip past the check (concurrent writes) propagate as errors — never string-match driver error messages.

**Why fragile:** the app-side check races with concurrent inserts (TOCTOU); only a driver-level predicate closes it.

**Clean upstream API:** `Error::is_unique_violation()` (or `ErrorKind::UniqueViolation { constraint }`) surfaced by the SQL drivers.

**Argentum plan:** keep `check_unique` as the UX layer; when the predicate lands, map it to the constrained field's inline error and delete this entry. Never string-match driver error messages.

---

## Topcoat — memoize(as_ref) error conversion

**Where:** `examples/showcase/src/pages/showcase/db.rs` `users()` — the only sanctioned stringification site.

**Today:** `#[memoize(as_ref)]` caches `Result<&T, &Error>`; `topcoat::Error` wraps `anyhow::Error`, which is not `Clone`, so a memoized fallible loader cannot hand back an owned `topcoat::Error`. The workaround converts via `std::io::Error::other(e.to_string())`, which erases typed predicates (`is_record_not_found`, …) at the memo boundary.

**Why fragile:** the pattern is easy to cargo-cult into non-memoized call sites (it was, once — removed 2026-08-26).

**Clean upstream API:** memoize supporting non-`Clone` error types — e.g. cache `Result<Arc<T>, Arc<Error>>` and return `Result<&T, &Arc<Error>>`, or an `Arc`-backed `Error` clone.

**Argentum plan:** keep the workaround localized to `db.rs` `users()`; non-memoized loaders use `.map_err(Into::into)`. Retire when memoize supports `Arc`'d errors.

---

## Topcoat — interpolating an opaque `impl View` (`NodeClassify` blanket impl)

**Where:** every Argentum helper boundary — the page handlers in `crates/argentum-core/src/panel.rs` (all return `BoxView<'_>`) and interpolation sites like `suspense(fallback: skeleton, (lazy_rows.boxed()))`; same shape in `argentum-ui` composites.

**Today:** `(expr)` interpolation requires `NodeClassify`, implemented only for five hard-coded types (`Child`, `BoxView`, `ScopeView`, `MoveView`, `LiveView`). A helper returning `Result<impl View>` yields an opaque that implements `View` but not `NodeClassify`, so it cannot be interpolated — every helper must return `BoxView` or be boxed at the call site. The error names the wrong trait and suggests nothing.

**Why fragile:** `internal` has no stability guarantee, and classification sits on Argentum's hottest path (every helper boundary). An upstream rename breaks all of it at once.

**Clean upstream API:** `impl<T: View> NodeClassify for T` (or classification without a trait), or failing that a diagnostic saying "box it with `.boxed()`".

**Argentum plan:** keep `BoxView` returns and `.boxed()` at interpolation sites — they are cheap anyway. Drop the boxes and this entry when the blanket impl lands.

---

## Topcoat — hand-registered fallible async pages (`ThenView` is internal)

**Where:** the single `use topcoat::view::internal::ThenView;` in `crates/argentum-core/src/panel.rs:10`; `Box::pin(HoistView::new(ThenView::new(async move { .. })))` in the seven resource handlers (`resource_list`/`resource_create`/`resource_create_post`/`resource_edit`/`resource_edit_post`/`resource_delete`/`resource_bulk_delete`) and the nested `lazy_rows` suspense child.

**Today:** `Panel` hand-registers pages through the public registry seam — `PageFn::new(method, path, handler)`; `PageRenderFn` is sync (`fn(&Cx, Body) -> BoxView`), so a fallible async page body (auth check → `Err(forbidden().into())`, awaits, `Ok(view! { .. })`) can only be expressed by adapting the future with the internal `ThenView` and boxing — the same adaptation `#[page]` performs internally, which also includes the public `HoistView` wrap: signals are ordinary `signal(cx, …)` calls now, and a body that creates one must run inside a `HoistView`.

**Why fragile:** `topcoat::view::internal` is explicitly unstable; a refactor there moves Argentum's central page adapter. There is no public alternative: nothing else converts a fallible future-of-a-view into a view.

**Clean upstream API:** a public constructor on the page side — e.g. `page_from_fn(.., |cx, body| async { Result<impl View> })` — or `pub use ThenView` (a future-of-a-view becomes a view; its `Err` becomes the view's error).

**Argentum plan:** keep the one internal import and the `ThenView`-boxed shape (it also powers the streamed `lazy_rows`); swap to the public constructor and delete this entry when it lands.

---

## Topcoat — no idiomatic list-of-views

**Where:** row rendering in `crates/argentum-core/src/resource.rs` (row loop with `key:` per row, `key:` per cell), the skeleton loop (`key: i`), the empty state (`key: "empty"`).

**Today:** dynamic lists render as `for` loops inside one `view!`/component call, with `key:` on every component call. `BoxView` is the only container and is documented only as the fix for multiple `return` sites; `Child` has no `FromIterator`; nothing documents whether `Vec<BoxView>` can be interpolated.

**Why fragile:** the loop pattern works and nothing breaks — but the rule is discoverable only by trial, and a future refactor toward pre-built view lists (`Vec<BoxView>`) has no documented support either way.

**Clean upstream API (any one):** `impl FromIterator<BoxView<'a>> for Child<'a>`, a `fragment!` helper, or a `view.md` note stating the intended pattern is a `for` loop inside one `view!` with `key:` on component calls.

**Argentum plan:** keep loop + `key:`; adopt whichever upstream shape lands and delete this entry.

---

## Topcoat — views may borrow only `cx` (undocumented borrow rule)

**Where:** every view-building site; representatives: `crates/argentum-core/src/panel.rs` (`prefix.clone()`, `title.clone()` captured into async page bodies) and `crates/argentum-core/src/resource.rs` (`(cell.clone())` per cell).

**Today:** views borrow the `Cx` they were built against, therefore a view can never borrow anything owned by the code that returns it. Capturing a handler local fails with E0515 `cannot return value referencing local variable`; Argentum clones at capture sites. The rule is permanent (it follows from the lazy model) but documented nowhere — each new page rediscovers it by fighting E0515.

**Why fragile:** a docs gap, not an API one — nothing breaks, but every page pays a defensive clone. The upstream fix is a paragraph, not code.

**Clean upstream API:** one paragraph in `view.md` — "templates may borrow `cx` freely; anything else they capture must be owned or borrowed from `cx`" — plus a worked example.

**Argentum plan:** keep the clones (cheap: `String`s and small vecs); delete this entry when the paragraph lands.

---

## Toasty — instance → field-value extraction (row keys, cells)

**Where:** `crates/argentum-core/src/resource.rs` — `Table::id` row-key closure and `TextColumn`'s projection closure.

**Today:** Toasty models are plain structs; the `Model`/`Field` traits expose paths and schema metadata but **no way to read a field value off an instance generically** (`Load` only goes `Value → model`). Argentum therefore requires the projection as a user-written closure:
- `Table::id(|u| u.id.to_string())` — the row key for `key:`-ed rows (row identity is mandatory; render errors without it).
- `TextColumn::for_lens(lens, |u| u.name.clone())` — the cell projection.

Typos in either closure fail at compile time — but every table/column repeats the field read.

**Why fragile:** nothing breaks (closures are typed); the cost is ergonomic repetition, and a silent mismatch between the lens (query side) and the closure (render side) cannot be detected — e.g. a column whose lens says `email` but whose closure reads `name` still compiles.

**Clean upstream API:**
```rust
trait Model {
    fn primary_key(&self) -> Self::PrimaryKey;        // row keys without the closure
    // and/or: generated per-field accessors usable as `Fn(&Self) -> &T`
}
```

**Argentum plan:** keep the closures until upstream exposes instance→value access; then default `Table::id` / `TextColumn` projections from the lens and delete this entry.

---

## Toasty — `IN` predicate for bulk id lists

**Where:** `crates/argentum-core/src/schema.rs` `pk_in_expr` (N-way `OR`), bounded by `MAX_BULK_IDS = 400` in `panel.rs` bulk-delete (GH #85).

**Today:** no `IN` combinator on the public facade for a dynamic id list, so bulk builds `pk == a OR pk == b …`. The cap keeps planner/URL pressure bounded.

**Clean upstream API:** `fn eq_any(Path, Vec<T>) -> Expr<bool>` (or `IN` combinator) usable from generic code holding parsed `stmt::Value`s.

**Argentum plan:** keep the capped `OR` chain; swap to `IN` when it lands and delete this entry.

---

## Retired entries

- **Topcoat error conversion** (fixed 2026-08-26, Argentum-side): `toasty::Error → anyhow → topcoat::Error` via `From`; canonical pattern is `.map_err(Into::into)` / `?`. The `db.rs` memoize site stays under its own entry.
- **Panel prefix vs `NavigationItem`** (fixed internally): `from_resource_with_prefix` + `Panel::nav_item` respect the mount prefix; `from_resource` is the `"/admin"` shorthand. No upstream gap.
- **Toasty PK tie-breaker for cursors** (retired 2026-09-04, GH #76): toasty appends physical PK columns to ambiguous cursor orderings internally; the app-level suffix was dropped.
- **`author include` / Phase-2 relations** (never an upstream gap): plain `include` + `computed`/`relationship` on the `Resource::query` seam; grouping/export are in-memory shims needing no upstream API.

---

## How to retire entries

1. Add the upstream API (or feature-flag it).
2. Grep for `toasty_core::` — should become zero outside the `schema.rs` bridge helpers and `cursor.rs` cursor values.
3. Update the bridge helpers to delegate to the new public API, keep signature.
4. Delete the entry here and reference the Toasty/Topcoat PR that closed it.
