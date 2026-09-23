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
            .columns((
                TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone())
                    .searchable()
                    .sortable(),
            ))
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

| Path | What it is |
| --- | --- |
| `crates/argentum-core` | `Panel`, the `Resource` trait, `Table` and `Schema` types, auth, tenancy |
| `crates/argentum-macros` | the `derive(EmbeddedForm)` macro for embedded values (GH #191) |
| `crates/argentum-ui` | Topcoat primitives plus owned composites (`Page`, `Toast`, `Theme`, `ErrorState`) |
| `examples/showcase` | runnable admin: `/admin/users`, `/admin/authors`, `/admin/posts`, `/admin/comments` |
| `benchmarks/` | server-render perf harness plus axum-maud and leptos smoke stubs |
| `docs/guide` | the user guide, an mdBook |
| `docs/adr` | one design note per decision |
| `docs/dev` | contributor specs: commits, prose, labels |
| `docs/agents` | process notes for agents: issue tracker, domain docs |
| `CONTEXT.md` | the domain vocabulary the code and the docs share |

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

## Roadmap

Done:

- CRUD for single resources: typed tables, cursor pagination, search and sort, create and edit
  forms, row and bulk delete, flash notifications, sidebar shell
- Relations: preloaded `BelongsTo` and `HasMany`, relation selects, tenant scoping, server-side
  option search past the cap
- Table extras: typed filters, page-local grouping with counts, CSV export, live in-place search and
  sort, empty and error states
- Auth: default login plus sessions, custom user table seam, explicit opt-out, per-resource policy
- Media uploads: an app-level `Uploader` seam, and `Panel::serve_dir` for the directory it writes to
  (ADR-0017)
- Media library (showcase): a polymorphic `medias` table, an app-level upload page, and a thumbnail
  or link per stored row (ADR-0021)

Next:

- Widgets and infolists: stats overview, charts, global search
- Image handling in the framework: transcoding, dimensions (the showcase renders a stored image at
  thumbnail size, ADR-0021)
- Documented production migrations

Non-goals for v1:

- Many-to-many helpers in tables
- DynamoDB-backed admin
- SQL aggregates beyond row counts
- WASM admin or SPA mode

## Contributing

Small fixes can go straight to a PR; larger changes are worth an issue first. Every branch is
**squash-merged into `master`** as one Conventional Commit (`<type>(<scope>): <description> (#123)`).
[`CONTRIBUTING.md`](CONTRIBUTING.md) covers the build, the CI gate set, and the commit rules;
[`AGENTS.md`](AGENTS.md) is the short version for agents. The vendored-primitives rule is worth
repeating here: `crates/argentum-ui/src/components/primitives/` mirrors `topcoat-ui-registry`
verbatim — never hand-edit it, sync with `cargo xtask sync-topcoat-ui` (ADR-0007). Composites
(`Page`, `Toast`, `Theme`, `ErrorState`) are Argentum's own.

## License

MIT — see [`LICENSE`](LICENSE).
