# External gaps — what Argentum works around

This file tracks **missing or unstable APIs in upstream crates** (Toasty, Topcoat) that force Argentum to reach into internals or duplicate logic. Each gap lists: what Argentum does today, what a clean upstream API would look like, and how to migrate when it lands. The goal is to make the workarounds visible and cheap to remove.

> **Policy:** Do not hesitate to use `toasty_core` / `topcoat_core` internals inside `argentum-core` / `argentum-ui` when the public API is missing. Every such use **must** be documented here as a gap with "Where / Today / Why fragile / Clean upstream API / Argentum plan" so it can be upstreamed — you (maintainer) contribute the fix to Toasty/Topcoat and retire the entry. Keep this file focused: only gaps that truly belong upstream. Internal choices (e.g. Tailwind `@source` for `argentum-ui`, now ADR-0006) do **not** belong here.
>
> Audience: contributors; not user-facing. Link to this file from `crates/argentum-core/src/schema.rs` and `crates/argentum-core/src/resource.rs` where the workarounds live. Keep it short — one entry per gap.

---

## Toasty — building `OrderByExpr` / naming core `stmt::Path` for a field lens

**Where:** `crates/argentum-core/src/schema.rs` `lens_field_name_and_label` and `pk_tie_breakers` (the two `toasty_core` import sites); consumers: `TextInput` (`schema.rs`), `TextColumn` / `Table::order_bys` (`resource.rs`).

**Today (accurate as of 2026-08-29):** Most of what this entry used to claim is **public already**: `toasty::schema` re-exports the whole app-schema surface (`crates/toasty/src/schema.rs:49` → `toasty_core::schema::{app, db, diff, mapping}`), so `M::schema()`, `Model::fields()`, `Field.name` (`FieldName::app_unwrap()`), `Field.primary_key`, and `ModelRoot::primary_key_fields()` are all reachable via the `toasty` facade without depending on `toasty-core`. Field-name resolution (`lens_field_name_and_label`) walks `core_path.projection.as_slice()[0]` → `M::schema().fields()[idx].name` — the `Projection` type is even re-exported as `toasty::stmt::Projection`.

What still genuinely requires `toasty_core`:
1. **Naming the conversion target.** `From<toasty::stmt::Path<M, T>> for toasty_core::stmt::Path` is public (`crates/toasty/src/stmt/path.rs`), but the core `stmt::Path` type itself is not re-exported through the facade, so holding the converted value needs the `toasty_core` path.
2. **Building PK order-bys.** There is no typed wrapper for `Expr::ref_self_field(field_id)` / `Direction::Asc`, so `pk_tie_breakers` must construct `toasty::stmt::OrderByExpr` from core `stmt` pieces.

**Why fragile:** only those two spots. A toasty refactor of `Path`/`Projection` breaks them at compile time; the rest survives.

**Clean upstream API:**
```rust
// ideal — makes both bridge helpers deletable
impl<M: Model> Path<M, T> {
    pub fn field_name(&self) -> &str;           // app-level name
    pub fn storage_column_name(&self) -> &str;  // db column name
    pub fn label(&self) -> String;              // capitalize(field_name)
}
trait Model {
    fn primary_key_order_bys() -> Vec<OrderByExpr>; // or Vec<Path<Self, _>>
}
```

**Argentum debt:** keep both helpers as the single `toasty_core` import sites. Follow-up #11 enriches `FieldLens` with `is_nullable`/`is_unique`/`column_name` — implement those from the **public** `toasty::schema::app` metadata, not via `toasty_core`. When upstream lands the two APIs above, replace the helpers and delete this entry.

---

## Toasty — primary-key tie-breaker for deterministic pagination

**Where:** `crates/argentum-core/src/schema.rs:pk_tie_breakers` and `crates/argentum-core/src/resource.rs:Table::order_bys`.

**Today:** `M::schema().as_root().primary_key.fields` (public via `toasty::schema::app`, see the entry above) → `toasty_core::stmt::Expr::ref_self_field(field_id)` → `OrderByExpr { asc }`. Appends PK asc to the user-chosen `order_by` for stable cursor pagination (spec US10).

**Why fragile:** only `Expr::ref_self_field` (and naming the core `stmt` types it returns) is unreachable from the `toasty` facade; the schema walk itself is public. See "Toasty — building OrderByExpr / naming core stmt::Path" for the full picture.

**Clean upstream API:**
```rust
trait Model {
    fn primary_key_paths() -> Vec<OrderByExpr>; // or Vec<Path<Self, _>>
    fn pk_order_bys() -> Vec<OrderByExpr> { Self::primary_key_paths().into_iter().map(|p| p.asc()).collect() }
}
 // or Table::order_bys delegates to Model::pk_asc()
```

**Argentum debt:** Centralised in `pk_tie_breakers` so only one file imports `toasty_core`. Follow-up #10/#13 will consume it when Toasty exposes PK paths; then `Table::order_bys` becomes `vec![first].extend(M::pk_order_bys())` with no `toasty_core`.

---

## Toasty — field metadata for Schema hydration

**Where:** `TextInput::validate` / future `Create`/`Update` hydration (`crates/argentum-core/src/schema.rs:TextInput`).

**Today:** `FieldLens` is just `Path<M,T>`; `TextInput` knows `required`/`is_email` but not `is_nullable`/`is_unique`/`column_name`. Validation manually checks `required` and hand-rolled `is_valid_email`. Follow-up #11 notes the gap.

**Clean upstream API:** Same as lens gap — `Path::is_nullable()`, `Path::is_unique()`, `Path::column_name()`, plus `FieldTy` so `TextInput::for(...).required()` can default from `field.nullable == false`.

**Argentum plan:** Keep `FieldLens = Path<M,T>` alias for now; don't add trait until Toasty exposes it. When it does, `FieldLens` becomes a trait `Lens<M,T>` with those accessors and `TextInput` derives defaults.

---

## Toasty — LIKE escaping helper

**Where:** `README.md §6` and future Table `like_with_escape`.

**Today:** Portable search uses `starts_with` (safe, parameterised). Substring search would need `like_with_escape` with manual `q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")` + `format!("%{esc}%")`.

**Clean upstream API:** `fn escape_like(pattern: &str, escape: char) -> String` or `Path::contains_escaped(q: &str)` that parameterises and escapes server-side.

**Argentum plan:** Keep `starts_with` for Phase 1; add helper only when a Filter needs substring search. Don't vendor an escape fn yet.

---

## Topcoat — error conversion

**Where:** `examples/showcase/src/pages/admin_list.rs`, `examples/showcase/src/pages/showcase/*.rs`, `examples/showcase/src/pages/showcase/db.rs`.

**Today (clean):** `toasty::Error` implements `std::error::Error + Send + Sync` → `anyhow::Error` → `topcoat::Error` via `impl<T: Into<anyhow::Error>> From<T> for topcoat::Error`. Correct pattern is `.map_err(Into::into)?` or `?` directly.

**Previous workaround (removed 2026-08-26):** `.map_err(|e| std::io::Error::other(e.to_string()))?` stringified the error, losing `is_record_not_found`/`is_unique_violation` predicates needed for spec §9 inline field errors and proper HTTP status mapping.

**Upstream gap:** None — Topcoat already supports `From`. Gap was Argentum-side misuse, now fixed. Keep `map_err(Into::into)` as the canonical pattern and avoid reintroducing stringification.

---

## Topcoat — Panel prefix vs NavigationItem

**Where:** `crates/argentum-core/src/panel.rs:Panel::navigation_item` and `crates/argentum-core/src/resource.rs:NavigationItem::from_resource_with_prefix`.

**Today (clean):** `NavigationItem::from_resource_with_prefix::<R>(prefix)` and `Panel::navigation_item::<R>(&self)` respect `Panel::prefix()`. `from_resource` remains as `"/admin"` shorthand for Phase-1 single-panel compat.

**Previous gap:** `NavigationItem::from_resource` hard-coded `url: "/admin"`, breaking `Panel::new("backoffice")`. No upstream gap — fix was internal.

---

## Toasty — unique-violation error predicate

**Where:** `README.md` §4.3/§9 (unique → inline field-error mapping); future Create/Update hydration (#11, #12). No runtime call site today — the mapping is aspirational.

**Today:** toasty exposes no unique-violation error kind. `toasty-core/src/error/` has `is_record_not_found`, `is_condition_failed`, … but no `is_unique_violation`; `#[unique]` only creates the DB index and duplicates surface as an unclassified driver error (the toasty quickstart example only asserts `dup.is_err()`).

**Why fragile:** any code or doc claiming to map `UniqueViolation` to a field error cannot be implemented without driver-specific string matching.

**Clean upstream API:** `Error::is_unique_violation()` (or `ErrorKind::UniqueViolation { constraint }`) surfaced by the SQL drivers.

**Argentum plan:** keep app-level validation the only error layer until the predicate lands; then map it to the constrained field's inline error and delete this entry. Never string-match driver error messages.

---

## Topcoat — memoize(as_ref) error conversion

**Where:** `examples/showcase/src/pages/showcase/db.rs` `users()` — the only sanctioned stringification site.

**Today:** `#[memoize(as_ref)]` caches `Result<&T, &Error>`; `topcoat::Error` wraps `anyhow::Error`, which is not `Clone`, so a memoized fallible loader cannot hand back an owned `topcoat::Error`. The workaround converts via `std::io::Error::other(e.to_string())`, which erases typed predicates (`is_record_not_found`, …) at the memo boundary.

**Why fragile:** the pattern is easy to cargo-cult into non-memoized call sites (it was, once — removed 2026-08-26 per the Topcoat error-conversion entry above).

**Clean upstream API:** memoize supporting non-`Clone` error types — e.g. cache `Result<Arc<T>, Arc<Error>>` and return `Result<&T, &Arc<Error>>`, or an `Arc`-backed `Error` clone.

**Argentum plan:** keep the workaround localized to `db.rs` `users()`; non-memoized loaders use `.map_err(Into::into)`. Retire when memoize supports `Arc`'d errors.

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

**Argentum plan:** keep the closures until upstream exposes instance→value access; then default `Table::id` / `TextColumn` projections from the lens and delete this entry. Follow-up #10 tracks the Argentum side.

---

## How to retire entries

1. Add the upstream API (or feature-flag it).
2. Grep for `toasty_core::` — should become zero outside `crates/argentum-core/src/schema.rs` bridge helpers.
3. Update the bridge helpers to delegate to the new public API, keep signature.
4. Delete the entry here and reference the Toasty/Topcoat PR that closed it.

Last updated: 2026-08-30 (instance→field-value extraction gap documented with the typed Table projection, GH #10). 2026-08-29: memoize error conversion + missing unique-violation predicate documented; showcase stringification corrected. 2026-08-28: Tailwind `@source` moved to ADR-0006 (internal), policy added: use internals freely and document missing public APIs here.
