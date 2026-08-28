# External gaps — what Argentum works around

This file tracks **missing or unstable APIs in upstream crates** (Toasty, Topcoat) that force Argentum to reach into internals or duplicate logic. Each gap lists: what Argentum does today, what a clean upstream API would look like, and how to migrate when it lands. The goal is to make the workarounds visible and cheap to remove.

> Audience: contributors; not user-facing. Link to this file from `crates/argentum-core/src/schema.rs` and `crates/argentum-core/src/resource.rs` where the workarounds live. Keep it short — one entry per gap.

---

## Toasty — typed field lens → name / label / column

**Where:** `crates/argentum-core/src/schema.rs:lens_field_name_and_label` (`FieldLens<M,T> = Path<M,T>` → `M::schema().fields()[idx].name`).

**Today:** `Path<M,T>` converts to `toasty_core::stmt::Path` (`path.into()`), then `projection.as_slice()[0]` yields the field index, then `M::schema().fields()[idx].name.app_unwrap()` gives the app-level name. Label is `capitalize(name)`. This walks `toasty_core::stmt::Path` and `toasty_core::schema::app::FieldName` internals.

**Why fragile:** `Projection` layout and `FieldName::app_unwrap()` are not part of Toasty's public `toasty::schema::Model` surface. A Toasty refactor of `Path`/`Projection` breaks Argentum at compile time without a deprecation.

**Clean upstream API:**
```rust
// ideal
impl<M: Model> Path<M, T> {
    pub fn field_name(&self) -> &str;           // app-level name
    pub fn storage_column_name(&self) -> &str;  // db column name
    pub fn label(&self) -> String;              // capitalize(field_name)
    pub fn is_nullable(&self) -> bool;
    pub fn is_unique(&self) -> bool;
}
 // or
impl<M: Model> Model for M {
    fn field_for_path<T>(path: &Path<M,T>) -> &Field; // returns Field with name/ty/nullable
}
```

**Argentum debt:** `FieldLens` alias only exposes `name`/`label`. Follow-up #11 tracks enriching it with `is_nullable`/`is_unique`/`column_name` once upstream exposes `FieldTy` metadata without going through `toasty_core`. When upstream lands, replace `lens_field_name_and_label` with `path.field_name()` and delete the `toasty_core` import.

---

## Toasty — primary-key tie-breaker for deterministic pagination

**Where:** `crates/argentum-core/src/schema.rs:pk_tie_breakers` and `crates/argentum-core/src/resource.rs:Table::order_bys`.

**Today:** `M::schema().as_root().primary_key.fields` → `toasty_core::stmt::Expr::ref_self_field(field_id)` → `OrderByExpr { asc }`. Appends PK asc to the user-chosen `order_by` for stable cursor pagination (spec US10).

**Why fragile:** `as_root()`, `PrimaryKey.fields`, `Expr::ref_self_field` are `toasty_core` internals. Public `toasty::schema::Model` does not expose `primary_key()` as typed `Path`s.

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

**Where:** `readme.md §6` and future Table `like_with_escape`.

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

## Topcoat — Tailwind `@source` for dependency scanning

**Where:** `examples/showcase/styles.css` (`@source "../../crates/argentum-ui/src/**/*.rs"`), `crates/argentum-ui/src/**/*.rs` (Token classes), `crates/argentum-core/src/resource.rs` & `schema.rs` (Table/Schema chrome uses Token classes).

**Today:** Per-app `styles.css` must explicitly `@source` the dependency crate's `src/**/*.rs` (`argentum-ui` and, for now, `argentum-core`) so Tailwind's scanner finds the Token utilities (`bg-background`, `border-border`, `text-muted-foreground`, etc.) used inside the library. This is the contract documented in ADR-0006: `examples/showcase` is the canonical reference, empty projects follow the docs (one `styles.css`, one `build.rs`) until a future `argentum new` scaffold automates them. `argentum-ui` is the single `@source` that must be added; `argentum-core` is also added for now because Table/Schema chrome currently uses token classes directly (raw HTML) rather than solely via `argentum-ui` primitives — once Table/Schema delegate fully to `argentum-ui` primitives, the core `@source` can be dropped.

**Why fragile:** Tailwind's default `source` detection walks `cwd` (the app) but not dependencies; scanning a dependency's source requires an explicit `@source` with a relative path that is brittle to workspace layout and not documented as a Topcoat public contract. A future Topcoat `tailwind` feature or `topcoat ui` scaffold could provide a helper that auto-discovers dependency sources (e.g., via `cargo metadata` or a `tailwind_build` that emits `cargo:rerun-if-changed` and `cargo:warning` for missing `@source`), removing the manual path.

**Clean upstream API:**
```rust
// ideal: Topcoat documents and provides a helper
// build.rs
topcoat::tailwind::BuildConfig::new()
    .discover_sources() // scans `cargo metadata` for `topcoat-ui` and `argentum-ui` crates
    .input("styles.css")
    .render()?;
// or
// styles.css
@import "tailwindcss";
@import "topcoat-ui/theme"; // auto-discovers sources
```

**Argentum plan:** Keep the explicit `@source` for `argentum-ui` (and `argentum-core` for now) in `examples/showcase/styles.css`; track here until Topcoat documents dependency scanning natively. When Topcoat provides a helper, replace the manual `@source` with that helper and delete this entry, but keep `EXTERNAL_GAPS.md` as the ADR-visible contract until then.

---

## How to retire entries

1. Add the upstream API (or feature-flag it).
2. Grep for `toasty_core::` — should become zero outside `crates/argentum-core/src/schema.rs` bridge helpers.
3. Update the bridge helpers to delegate to the new public API, keep signature.
4. Delete the entry here and reference the Toasty/Topcoat PR that closed it.

Last updated: 2026-08-28 (scaffold argentum-ui + Tailwind seam, #15).
