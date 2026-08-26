# Stable state after Slice 1-3 (typed lens → TextInput + TextColumn)

Date: 2026-08-26 — Status: accepted — Supersedes: ADR-0001 len seam is now proved end-to-end.

## Context

Slices 1-3 (#7/#8/#9) of #6 shipped the typed lens vertical slice: `TextInput::for(User::fields().name()).required().email()` via `Schema::new`, `TextColumn::for(...).searchable().sortable()` via `Table::for`, `Resource::table/form` as canonical entry points, `/admin` rendering via `Table` with `q` filter + deterministic `order_bys()` (PK tie-breaker), showcase `/admin/showcase/schema`+`/admin/showcase/table`.

Review of `origin/master...HEAD` (2026-08-26) flagged 5 Standards smells + 6 Spec partials. For a stable baseline before new features (Select, filters, pagination, shard) we need a clean tree where remaining gaps are *explicit* tech debt, not hidden drift.

## Decision

Tag stable at `055d2f7`..`2dfa906` (6 slices + 3 fix commits). What is deferred vs. proved:

- **Proved and stable:** `FieldLens = Path<M,T>` alias, `lens_field_name_and_label` centralised (`schema.rs:139`), `HasId`/`GetField` string dispatch stays but is documented (`resource.rs:186`), `order_bys()` PK tie-breaker centralised in `schema::pk_tie_breakers` (`schema.rs:182`, `resource.rs:275`), `showcase/table.rs:8` respects `Resource::query(cx)`, `UserResource::form` proves `Resource::form` seam (`showcase/app.rs:36`), showcase `/admin/showcase/*` all 200, `cargo test` 11 showcase + 30 core green.
- **Intentionally deferred (not fixed now):**
  1. **Typed Table projection** (S3/P9) — `Column` holding `Fn(&M)->String` instead of `HasId/GetField` — needs design, tracked as #10.
  2. **FieldLens metadata + email validator** (P8) — `is_nullable/is_unique/column_name` + `validator` crate — needed for Create/Edit hydration, tracked as #11.
  3. **Inline Schema errors inside `ac-field`** (P3/US18) — per-field error slots + `#[procedure]` — tracked as #12.
  4. **q-aware memoize + FilterBuilder + User role/active/index** (P1/P2) — `#[memoize]` with `q` key + cursor pagination + realistic `User` — needs shard/boundary, tracked as #13.
- **Ignored by design:** `IntoColumns`/`IntoSchema` 4-tuple limit (S2), `Column::Text` single variant (S5), showcase >70 LOC (US17) — documented in code, no ticket.

New seams introduced sit on highest existing seams (`Resource::table/form`, `Schema::new/render`, `Table::for`, `db(cx)`/`#[memoize]`), exactly one seam per feature surface per Further Notes #6.

## Consequences

- `cargo test --workspace && cargo clippy && cargo fmt --check` must stay green on stable.
- Review at this tag reports only documented debt (see 4 tickets). No new `toasty_core` usage outside `schema.rs:139/182`.
- Next feature (Slice 4: filters/pagination/shard) must consume tickets #10-#13 in order, not reintroduce hand-rolled `<table>` or string `statePath`.
