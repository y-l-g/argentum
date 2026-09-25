# Argentum

> **Admin toolkit for Rust**, server-rendered on **Topcoat** (UI and reactivity) and **Toasty** (ORM). Filament-style Panel plus Resource, tables, and forms, with no SPA build step.

[![CI](https://github.com/y-l-g/argentum/actions/workflows/ci.yml/badge.svg)](https://github.com/y-l-g/argentum/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## What it is

- Server-rendered HTML with `view!` and `#[component]`. No SPA, no WASM bundle.
- Typed end to end: Toasty model to query to table and form. A bad column name fails to compile.
- A Topcoat app: layouts, `href!`, `Cx`, `#[memoize]`, small `$(...)` expressions.
- Fast by default: concurrent renders, preloaded relations, cursor pagination.

It is deliberately not:

- Not a Livewire port. No string state paths, no reflection DI, no Blade partials.
- Not driver-agnostic in v1. The workspace targets Toasty over SQLite.
- Not a client framework. Anything that needs the DB renders on the server.

## Quick start

Run the showcase:

```sh
cargo run -p showcase
# open http://localhost:3000/admin/users
```

Define a resource, register it on a panel, delegate the layout:

```rust
pub struct UserResource;

impl Resource for UserResource {
    type Model = User;
    fn can_view_any(_cx: &Cx) -> bool { true }
    fn table(cx: &Cx) -> Table<User> {
        Table::r#for(cx)
            .id(|u: &User| u.id.to_string())
            .columns(
                TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone())
                    .searchable()
                    .sortable(),
            )
            .paginate(20)
    }
}
```

List-only minimal: writes stay 403 and create/edit render empty until you add
`can_create` / `can_update` / `can_delete`, `form()`, and the
`create_record` / `update_record` fns (see the [resources chapter](docs/guide/src/resources.md)).

```rust
#[layout("/admin")]
async fn admin_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    Panel::layout_shell(cx, slot).await
}

fn router(db: toasty::Db) -> Router {
    Panel::new("admin")
        .app_context(db)
        .resource::<UserResource>()
        // Minimal example: auth off. With default auth on, register
        // `AdminUser` + `AuthSession` in `toasty::models!` instead
        // (see the policy, auth, tenancy chapter).
        .auth(Auth::disabled())
        .build().expect("panel builds")
}
```

See `examples/showcase/src/app.rs` for the full version with forms, filters, and tenancy.

## Layout

Crate roles live in [`docs/dev/architecture.md`](docs/dev/architecture.md#crates). The user
guide is [`docs/guide/`](docs/guide/) (mdBook), decisions are in [`docs/adr/`](docs/adr/),
contributor specs in [`docs/dev/`](docs/dev/), domain vocabulary in
[`CONTEXT.md`](CONTEXT.md), and the runnable reference in
[`examples/showcase/`](examples/showcase/).

## Documentation

- **Guide**: [`docs/guide/`](docs/guide/) — build with `mdbook build docs/guide`, or read the
  published copy at <https://y-l.fr/argentum/nightly/guide/>. The rustdoc reference is published
  beside it at <https://y-l.fr/argentum/nightly/api/argentum_core/>.
- [`CONTEXT.md`](CONTEXT.md) — the vocabulary.
- [`docs/adr/`](docs/adr/) — the decisions, one note each.
- [`examples/showcase/`](examples/showcase/) — the runnable reference.
- [`benchmarks/README.md`](benchmarks/README.md) — the perf harness setup.
- Upstream: the [Toasty guide](https://tokio-rs.github.io/toasty/0.10.0/guide/) (queries, filters,
  sorting, preloading, migrations) and the [Topcoat docs](https://docs.rs/topcoat) (`view!` and
  `#[component]`, router, cookie and session).
- The [Filament PHP docs](https://filamentphp.com/docs) are product inspiration, not API source.

## Contributing

Small fixes can go straight to a PR; larger changes are worth an issue first.
[`CONTRIBUTING.md`](CONTRIBUTING.md) covers the build, the [CI gate set](CONTRIBUTING.md#the-gate-set),
and the commit rules; [`AGENTS.md`](AGENTS.md) is the short version for agents.

## License

MIT — see [`LICENSE`](LICENSE).
