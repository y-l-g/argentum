# Single-resource CRUD — Create/Edit, record fns, Policy, Notification

Date: 2026-08-31 — Status: accepted — Amended: 2026-09-10, 2026-09-15, 2026-09-21, 2026-09-22

## Decision

A `Resource` registered on a `Panel` is fully writable on the framework's existing seams.

- **Panel** owns the resource routes — the list page at `{prefix}/{slug}`, `GET`/`POST`
  `{prefix}/{slug}/create`, `GET`/`POST` `{prefix}/{slug}/{id}/edit`, `POST`
  `{prefix}/{slug}/{id}/delete`, `POST` `{prefix}/{slug}/bulk-delete`, `GET`
  `{prefix}/{slug}/export` and `GET` `{prefix}/{slug}/options` — plus the shell-level notification
  stack. `Router::builder().discover().cookies()` installs the cookie layer, and
  `Panel::layout_shell` renders the complete document.
- **Mutations** are `Resource` record fns called by the handlers inside a framework-owned
  transaction, with `can_*` re-checked on the tenant-scoped loaded snapshot; a bulk delete re-fetches
  every id through the same query and is all-or-nothing (ADR-0004). The panel's list loader wires the
  chrome the resource declares (`Resource::deletable` → row Delete + bulk bar, `Resource::editable`
  → Edit link, `Resource::viewed` → View link) with `Table::with_delete`, `Table::with_edit`,
  `Table::with_view` and `Table::with_bulk_delete`.
- **Schema** hydrates and dehydrates through typed lenses:
  `TextInput::r#for(User::fields().name()).required().email().unique()` fails to compile on a bad
  field; `Schema::hydrate` fills `value` attrs from the Model through `Resource::hydrate_form_values`,
  and validation collects `required`/`email` inline per field in the reserved destructive slot. Unique
  is checked app-side until Toasty exposes a unique-violation signal, so a concurrent write that
  violates the index still surfaces as a 500 (GH #88, open). `TextInput::unique()` implies **presence**
  (GH #189): an empty unique field reports `"<Label> is required"` inline, `.optional()` does not lift
  it, and `Panel::build` refuses a `.unique()` marker on a column with no unique index.
- **Notification** is a transient status + title (`success`/`error`, ~4s) produced by a mutation's
  result and rendered in the shell's top-level stack so it survives table swaps. It travels as a
  one-time flash cookie on a `303 See Other` redirect — `__Host-argentum_notification`, carrying
  `Secure` like the session and CSRF cookies, holding Topcoat's `CookieStore` JSON — so following the
  redirect consumes it and a reload never replays it. A page can also mount one in place
  (`notification::live_toast` plus the shell's `live_toaster`).
- **Table** parses `?q=`/`?sort=`/`?dir=`/`?after=`/`?before=` into `TableState` once. The declared
  default ordering and `?sort=` resolution are one entry point,
  `Table::order_bys_for(state, OrderMode)`, whose mode selects the list or export PK fallback, and
  `Table::apply_declaration` is the one routine that turns search, filters and ordering into a query
  for both loaders. The live shard is the slug-dispatched `table_search` behind `Table::live_search`,
  fed by the page's `TableSignals`. Row identity in `view!` loops is a loop-level `#[key(...)]` taken
  from the row key, never the loop index (GH #124).

## Consequences

`{prefix}/{slug}` supports search/sort/paginate/create/edit/delete/bulk-delete, all policy-checked,
with no N+1. `Panel` remains the single owner of Router/Db/Shell; `Resource::query` is the resource's
own row-scoping seam with the framework applying the tenant half (ADR-0002); `Schema` stays the form
seam and `Table` the list seam. No `Resource` hand-rolls `#[shard]`. The showcase test modules cover
the vertical slice, and `cargo test --workspace` / `clippy -D warnings` / `fmt` stay green per commit.
