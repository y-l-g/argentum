# Introduction

What Tablo is, what it deliberately is not, and the two upstream projects it is built on.

Tablo is an **admin toolkit for Rust**, server-rendered on
[Topcoat](https://github.com/tokio-rs/topcoat) (UI and reactivity) and
[Toasty](https://github.com/tokio-rs/toasty) (ORM): a Filament-style `Panel` plus `Resource`,
tables, and forms, with no SPA build step.

## What it is

- Server-rendered HTML with `view!` and `#[component]`. No SPA, no WASM bundle.
- Typed end to end: Toasty model to query to table and form. A bad column name fails to compile.
- Fast by default: concurrent renders, preloaded relations, cursor pagination.
- A Topcoat app: layouts, `href!`, `Cx`, `#[memoize]`, small `$(...)` expressions.

## What it is not

- Not a Livewire port. No string state paths, no reflection DI, no Blade partials.
- Not driver-agnostic in v1. The workspace targets Toasty over SQLite.
- Not a client framework. Anything that needs the DB renders on the server.

## The two upstream dependencies

Tablo is a thin, opinionated layer, not a framework of its own:

- **Topcoat** owns rendering, routing, request context, reactivity, assets, cookies and sessions.
  Tablo adds the admin-shaped pieces on top — `Panel`, `Resource`, `Table`, `Schema` — and
  follows Topcoat's idioms (`view!`, `#[component]`, `Cx`, `href!`, `#[memoize]`) rather than
  inventing parallel ones.
- **Toasty** owns the data layer: models, queries, filters, sorting, preloading and migrations.
  Tablo queries Toasty directly, so the typed model is the single source of truth for columns,
  relations and nullability.

Both track `main` and are pinned by `Cargo.lock`; bump them deliberately, never with a blanket
`cargo update`.

## How to read this guide

- [Panel and routing](./panel-and-routing.md): mounting a panel and the routes a resource serves.
- [Resources](./resources.md): the `Resource` trait, its contract, and tenancy.
- [Tables](./tables.md), [Forms](./forms.md), [Detail pages](./detail-pages.md): the three views.
- [Policy, auth, tenancy](./policy-auth-tenancy.md): who may see and change what.
- [Data access](./data-access.md): querying Toasty from panel code.
- [Security](./security.md) and [Testing and benchmarks](./testing-and-benchmarks.md): defaults and
  the tooling around them.

## Where this guide and the code disagree

The vocabulary lives in `CONTEXT.md`, the decisions live in `docs/adr/` (one ADR per decision, and
the ADRs record the *why*), and the runnable reference lives in `examples/showcase/`. Where this
guide and the code disagree, the code and `docs/adr/` win — please open an issue or a PR so the
guide catches up.
