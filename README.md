# Argentum

> **Filament for Rust** — a server-rendered admin toolkit on **Topcoat** (UI / reactivity) and **Toasty** (ORM).

Status: **early implementation** — three crates exist (`argentum-core`, `argentum-macros`, `argentum-ui`) plus a showcase example. Shipped today: the typed-lens vertical slice (ADR-0005), declarative `Panel` resource routes and navigation (ADR-0008), a real list view — search toolbar, sort links, cursor pagination, honest empty states (ADR-0009 shell) — and the `db(cx)`/`#[memoize]` glue. Not shipped yet: Action, Policy, Notification, Create/Edit hydration, the shard/boundary reactivity seam (tracked in GH issue #38). The sections below mix shipped design with the original spec; where they disagree, the **code and `docs/adr/` win** — this document is being brought back in sync (GH issue #53).

---

## 1. What it is / is not

**Is:**

- A server-side monolith. HTML is rendered on the server with `view!` / `#[component]`. No SPA, no client build, no WASM bundle.
- Type-safe: Toasty models → typed queries → typed tables and forms. Name a column that does not exist and it fails to compile.
- Fast by default: concurrent rendering + per-request memoization + explicit `include` preloading + `boundary` diffing. No hidden N+1.
- A Topcoat citizen: layouts, `href!`, `Cx`, `#[memoize]`, `topcoat-ui` (vendored), `$(...)` expressions — not a parallel framework.

**Is not:**

- Not a Livewire port. No string `statePath`, no reflection DI, no `Macroable`, no Blade partials. Those patterns are PHP ergonomics; Rust has better ones.
- Not driver-agnostic at v1. Postgres/SQLite first. DynamoDB and `via` many-to-many are explicitly SQL-only and out of scope for tables.
- Not a client framework. `$(...)` and signals are a tiny JS vocabulary. Anything that needs the DB is a server render.

---

## 2. Principles

These are invariants. Code that violates them is a bug.

1. **Pure renders.** Pages, layouts, and components are side-effect free and deterministic. No `HashMap` iteration in a `boundary`, no `Utc::now()` inside one, no random ID per render. Required for concurrent rendering, boundary hashing, and streaming re-renders.
2. **One truth for data.** Components query Toasty directly. No REST layer between UI and DB. `Db` lives in `app_context` and is cloned per request (`app_context::<Db>(cx).clone()` → `&mut db` for `exec`). See `demos/coffee-shop/src/models.rs` for the canonical glue.
3. **Server inputs are untrusted.** Every `#[shard]`, `#[procedure]`, and (soon) signal value comes from the client. `require_admin(cx).await?` (or tenant/policy check) runs **inside** every shard/procedure — layout guards do not cover shard endpoints.
4. **Explicit is fast.** `include` for relations, `#[index]` for filter columns, `#[memoize]` for shared loads, `boundary` around skeletons. No magic preloading, no implicit scans.
5. **Composable `Cx`, not middleware.** Tenant, locale, auth are `cx.with(Tenant(id))` scoped values and `fn require_*(cx: &Cx)` helpers (`topcoat/docs/functions_not_middlewares.md`). No Tower layer for business logic.

---

## 3. Architecture

```
argentum/
  argentum-core/      // Panel, Resource trait, Table/Schema/Action types, navigation, policy hooks
  argentum-macros/    // #[derive(Resource)] (grammar + macro crates)
  argentum-ui/        // optional: vendored topcoat-ui wrappers (table chrome, dialog, skeleton) — copied via `topcoat ui add`
```

Only `argentum-core` + `argentum-macros` are hard dependencies. UI chrome lives in **`argentum-ui`** (ADR-0006/0007): its *primitives* are a verbatim mirror of `topcoat-ui-registry`, synced by `cargo xtask sync-topcoat-ui` and never hand-edited; its *composites* (Sidebar, Page, CodeBlock) are owned Argentum components. Apps depend on the crate — they do not run `topcoat ui add`.

Topcoat stack assumptions (tracking `topcoat@main`): `view!` / `#[component]` (concurrent), `#[page]` / `#[layout]` / `module_router!` + `href!`/`rewrite`/`sitemaps`, `Cx` with `cx.with(...)` + `#[memoize]` (128-bit, no `Clone`/`Eq`), `cookie`/`session`, `asset!`, `tailwind`. Runtime (`signal`/`$(...)`/`#[shard]`/`#[procedure]`) is used as documented in `topcoat/docs/runtime.md` and `topcoat-runtime/macro/docs/shard.md`. Streaming `defer`+`boundary` and Signals v2 are **designs** (`SIGNALS.md`, `DESIGN.md`/`DESIGN-2.md`) — Argentum must work on today's `#[shard]` and migrate without rewriting every resource (see §7).

---

## 4. Core vocabulary

### 4.1 `Panel`

The admin app. Mirrors Filament's `Panel` builder but as Rust values.

```rust
Panel::new("admin")
    .app_context(db)
    .assets(AssetBundle::load().expect("generated assets"))
    .shell_assets(tailwind::stylesheet!(), GEIST)
    .resource::<UserResource>()
    .build() // -> Router via discover + app_context
```

`Panel` owns the `Router`, registers `Db` and declared resources in `app_context`, registers each resource list at `/{prefix}/{slug}`, and redirects the panel root to the first resource. An app registers the shell with one layout handler: `Panel::layout_shell(cx, slot).await`. Custom pages can add a typed `NavigationItem::from_href` through `Panel::navigation(..)`.

### 4.2 `Resource`

The mapping from a Toasty model to its admin UI. One resource = one model = a set of pages (list / create / edit / view). Heavily inspired by `filament/packages/panels/src/Resources/Resource.php` but typed.

```rust
pub trait Resource: Sized + Send + Sync + 'static {
    type Model: toasty::schema::Model + Send + Sync + 'static;

    fn slug() -> String { /* Filament-style: UserResource -> users */ }
    fn navigation_label() -> String { /* plural model name */ }
    fn query(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Self::Model>> {
        toasty::stmt::Query::<toasty::stmt::List<Self::Model>>::all()
    }
    fn table(cx: &Cx) -> Table<Self::Model>;
    fn form(cx: &Cx) -> Schema;
    fn pages() -> Pages<Self> { Pages::crud() }
    fn navigation() -> NavigationItem { NavigationItem::from_resource::<Self>() }
}
```

*Derive macro:*

```rust
#[derive(Resource)]
#[resource(model = User)]
struct UserResource;

#[derive(Resource)]
#[resource(model = User, query = only_ada)]
struct ScopedUserResource;
```

The derive implements `Resource` for the annotated type with the given `Model` and optional `query` override. It does **not** generate stringly-typed `field!("email")` — lenses are `User::fields().email()` (`toasty-macros/model/expand/fields.rs`).

`Panel::resource::<UserResource>()` registers the resource list at `/admin/users` (the slug is derived from the resource type: `BlogPostResource` becomes `/admin/blog-posts`) and derives the matching sidebar item. `Resource::query` is the single tenancy seam: `fn query(cx: &Cx) -> Query<List<User>> { Query::<List<User>>::all().filter(User::fields().tenant_id().eq(tenant_id(cx))) }` via `cx.with(Tenant(id))`. CRUD pages remain a target of `Pages`.

### 4.3 `Schema` (forms + infolists)

One primitive for both. Filament v4 unified `forms` into `schemas` (`packages/schemas/src/Schema.php`); Argentum does the same.

```rust
Schema::new((
    Section::new("Account").schema((
        TextInput::for(User::fields().email()).required().email(),
        Select::for(User::fields().role()).options(Role::variants()),
    )),
    Grid::new(2).schema((
        TextInput::for(User::fields().name()).required(),
        Toggle::for(User::fields().active()),
    )),
))
```

- Every field takes a **typed lens** `FieldPath<Model, T>`, not a string. The lens knows `Nullable`, `Unique`, `Deferred`, and `#[column]` renames.
- Layout primitives: `Section`, `Group`, `Grid(12)`, `Tabs`, `Wizard`, `Flex`, `Fieldset` — mirrors `filament/packages/schemas/src/Components/{Section,Grid,Tabs,Wizard}.php`.
- State is the Toasty model itself (or a `Create`/`Update` projection). No `data.foo.bar` string path. Hydration is `model -> Schema`, dehydration is `Schema -> Update<'a>` / `Create` + validation.
- Validation is **app-level** (`required`, `email`, `unique`, `exists`, `regex`) collected per-schema and returned inline. Toasty column constraints (`#[unique]`, type) are DB-level; toasty does not expose a unique-violation error predicate yet (see `EXTERNAL_GAPS.md`), so DB violations cannot map to field errors today — until then, uniqueness is checked app-side.

### 4.4 `Table`

Composition, not a 30-trait God object (`filament/packages/tables/src/Table.php`). `Table<M>` is a value describing the list view; Argentum renders it.

As shipped today (compiles — see `examples/showcase/src/app.rs` for the full resource):

```rust
Table::r#for(cx)
    .id(|u: &User| u.id.to_string())       // mandatory typed row key
    .columns((
        TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone())
            .searchable()
            .sortable(),
        TextColumn::r#for(User::fields().email(), |u: &User| u.email.clone())
            .searchable(),
        TextColumn::computed("Status", |u: &User| {
            if u.active { "Active" } else { "Inactive" }.to_string()
        }),
    ))
    .paginate(2)                           // real cursor pagination (?after=/?before=)
```

Columns/filters declare **how to query**, not just how to render: `searchable()` marks a column for portable `starts_with` search, `sortable()` requires an `order_by` mapping (PK tie-breaker appended for deterministic cursors), and the URL is the one truth — `?q=`, `?sort=`, `?dir=`, `?after=`, `?before=` parsed once into `TableState`. Filters (`SelectFilter`/`TernaryFilter`), `BadgeColumn`, bulk actions and `.copyable()` remain spec-level (GH #13/#38).

UI chrome: `Table` renders into `argentum-ui` (synced `topcoat-ui-registry`) `table` + `pagination` + `skeleton` primitives (skeleton shown via `defer`/`boundary` while loading — §7).

### 4.5 `Action`

Modal or inline operation backed by `#[procedure]`. Covers Filament's `Action/BulkAction/CreateAction/EditAction/DeleteAction` (`filament/packages/actions/src/Action.php`).

```rust
Action::make("delete")
    .requires_confirmation()
    .authorize(|cx, record: &User| cx.can_delete(record))
    .form(|f| f.schema((Placeholder::new("name", &record.name),)))
    .action(|cx, record: User| async move {
        record.delete().exec(&mut db(cx)).await?;
        Ok(Notification::success("Deleted"))
    })
```

- Runs via `#[procedure]` endpoint, so same trust rule as shards: authorize inside the handler against the *passed* identity, plus DB transaction.
- Built-ins: `CreateAction`, `EditAction`, `DeleteAction`, `BulkDelete`, `Export`. Each maps to a `Schema` (modal form) when needed.

---

## 5. Pages, routing, navigation

- **Routes:** `Panel::resource::<UserResource>()` registers `GET /admin/users` and `Panel` redirects `GET /admin` to the first declared resource. Additional `#[page]` handlers are discovered normally. CRUD routes remain a target.
- **Layouts:** An app's `#[layout("/admin")]` handler delegates to `Panel::layout_shell(cx, slot)`. The shell owns the complete document, sidebar, runtime scripts, and links to the app-provided `tailwind::stylesheet!()` and `fontsource_font!(.., host: Asset)` handles.
- **Navigation:** `Panel::resource` derives one item per resource from its slug; `Panel::navigation(NavigationItem::from_href(..))` adds typed links for custom pages. Resource items use exact paths plus slash-boundary subpages for active state.
- **Errors & redirects:** `Panel::layout_shell` currently propagates `slot` errors unchanged. Tables propagate `?`; branded empty states are rendered by `Table`, while a dedicated `ErrorState` for load failures is deferred (see `CONTEXT.md`).

---

## 6. Data layer — Toasty

### 6.1 The seam

```rust
fn db(cx: &Cx) -> Db { app_context::<Db>(cx).clone() }

// in a component/shard/procedure:
let mut db = db(cx);
let rows = User::all().exec(&mut db).await?;
```

`Db` is `Arc`-pooled (`toasty/src/db/pool.rs`). `exec` needs `&mut Db` — clone per request is cheap. Transactions: `let mut conn = db(cx).connection().await?; let mut tx = conn.transaction().await?;`.

### 6.2 How tables and searches query

There is **no** `User::find().where(User::name.contains(&q)).all(cx.db())`. That API does not exist. The real API (`toasty/src/stmt/query.rs`, `docs/guide/filtering-with-expressions.md`):

```rust
// exact:
User::filter(User::fields().email().eq("alice@example.com"))
// prefix (portable):
User::filter(User::fields().name().starts_with(q.clone()))
// substring (SQL only):
User::filter(User::fields().name().like(format!("%{escaped}%")).escape('\\'))
// case-insensitive (PG only: ilike; others need collation or app-side lower):
User::filter(User::fields().name().ilike(format!("%{q}%")))
```

- `starts_with` is the only portable string predicate. `like` is SQL-only, `ilike` is Postgres-only (`Capability::native_ilike`). `contains` on `String` does not exist (confusable with `Vec<String>::contains` for collections — `stmt/path.rs:495`).
- `like` patterns must escape `%`/`_` from user input via `like_with_escape`.
- Global search across `searchable()` columns is `Expr::or` over each column's predicate; split terms are `and`ed per term (mirrors `filament HasGlobalSearch.php`).

### 6.3 Filters, sorting, pagination

- **Dynamic filters:** Toasty has no `Condition::add_option` yet (`docs/dev/roadmap.md`). Build queries imperatively:
  ```rust
  let mut q = Resource::query(cx);
  if let Some(term) = search { q = q.filter(col.like(...)); }
  for f in active_filters { q = q.filter(f.to_expr()); }
  ```
  Argentum's `FilterBuilder` wraps this (`Expr::and_all`).
- **Sorting:** `order_by(col.asc()/desc())`, tuples or chained calls. Spec's `Sort` must be mapped: `match sort { Sort::NameAsc => User::fields().name().asc(), ... }`.
- **Pagination:** Cursor pagination, **requires** `order_by` (`toasty/docs/guide/sorting-limits-and-pagination.md`). `paginate(per_page)` returns `Page<M>` (upper bound, `has_next()` not `len`). Walk `while let Some(n)=page.next(&mut db).await?`. Recent fix appends PK tie-breaker for determinism (`#1142`) and preserves cursors through `include` (`#1152`). `limit`/`offset` exists but `offset` requires `limit` and is slow at large offset — prefer cursor.

### 6.4 Relations & N+1

- Use `Deferred<Vec<Post>>` / `Deferred<User>` and preload with **one round-trip**:
  ```rust
  User::all().include(User::fields().posts()).exec(&mut db).await?;
  // then `user.posts.get().len()` — no await
  ```
  Eager `Vec<T>` is for tiny relations only — eager cycles are a compile-time schema-build error (`toasty-macros/model/schema/has_many.rs`). Spec's “all relations must be eager” is inverted.
- `has_many(via = memberships.group)` (many-to-many via join model) is **SQL-only, read-only** (`docs/guide/many-to-many.md`). Out of scope for v1 tables; use the join model query and deduplicate when DynamoDB compatibility is desired.
- Every `include` is `NestedMerge` + correlated subquery (`engine/lower/include.rs`). Shared prefix includes deduplicate.

### 6.5 Aggregates & dashboards

Toasty has `count()` but no `GROUP BY / HAVING / SUM / DISTINCT` yet (`roadmap #421/#425`). Dashboard cards use raw SQL helper:

```rust
let rows: Vec<ValueRecord> = toasty::sql::query("SELECT status, COUNT(*) FROM orders GROUP BY status")
    .exec(&mut db).await?;
```

Hide behind `trait Aggregate` so tables don't leak raw SQL.

### 6.6 Schema & migrations

`Db::builder().models(toasty::models!(crate::*)).connect(url).await?; db.push_schema().await` for POC. Prod uses `embed_migrations!()` (`toasty/src/migration/embed.rs`) + `history.toml` + `snapshots/*.toml` via `toasty-cli`. Async pool is `num_cpus*2`, health-sweep, `pre_ping` (`db/pool.rs`).

---

## 7. Reactivity — today's API and the seam that survives

Today on `main` (`topcoat/docs/runtime.md`, `topcoat-runtime/macro/docs/shard.md`):

```rust
signal query = String::new(); // view!-local, client-only; emits <!--::topcoat::signal-->
<input :value=$(query.get()) @input=$(|e: Event| query.set(e.target.value))>
results(query: $(query.get())) // #[shard] async fn results(cx:&Cx, query:String)->Result
```

- Reads in `$(...)` re-run in JS with no server round-trip. Shard args are `$(...)` expressions; on change the shard POSTs to `/_topcoat/shards/<id>` (`runtime/src/shard.rs`) and swaps HTML. In-flight requests are aborted, same-tick changes coalesced. Re-render **resets** shard-local signals.
- Guards on page/layout **do not run** on shard requests — the shard must authorize itself.

Design direction (`SIGNALS.md` #335, `DESIGN.md`/`DESIGN-2.md` #332, `DESIGN_DELTA.md`) keeps the same *contract* but changes the mechanism:

- `let q = signal(|| String::new())` — function, stable identity via `#[track_caller]` + component stack (no positional comment, no hook ordering). `q.get()` in plain Rust registers as a server dependency.
- When that dependency changes, the client refetches the page (or shard subset) with `X-Topcoat-State` + `X-Topcoat-Boundaries`; server re-renders; `boundary` diff (`<!--topcoat-boundary id-->`) ships only changed regions.
- `defer(cx)` / `defer(cx, future)` + `boundary` for streaming skeletons: first pass ships shell, later passes swap real content, all via `#[memoize]`.

**Argentum's contract (works on `main`, migrates without rewriting resources):**

- Tables and slow cards are **`boundary()` regions**. Argentum exposes `Table::boundary(true)` (default) and `Card::defer(true)`.
- Filter/search/sort/page state are **signals owned by the page**. *How* they trigger a server render (today: `#[shard]`, tomorrow: page refetch + boundary diff) is an **internal detail** of `ArgentumTable`. Resources never hand-roll `#[shard]`.
- All data loads that may be deferred are **`#[memoize]`d** (`topcoat-core/macro/docs/memoize.md`): `#[memoize(as_ref)] async fn load_rows(cx:&Cx, q:String, sort:Sort) -> Vec<Row>`. This makes streaming, concurrent rendering, and fan-out dedup free (`try_join!` of sibling components shares one future).

Today's live-search shape (Phase 1) — future shape in comment:

```rust
// Phase 1 — compiles on main
#[component]
async fn users_page(cx: &Cx) -> Result {
    view! {
        signal q = String::new();
        <input :value=$(q.get()) @input=$(|e: Event| q.set(e.target.value))>
        user_table(query: $(q.get())) // user_table is #[shard] internally, owned by Argentum
    }
}
// Future — same call site, no hand-rolled shard (ArgentumTable hides the change):
// let q = signal(|| String::new());
// view! { <input :value=$(q.get()) @input=$(|e| q.set(e.target.value))> boundary(user_table(cx, &q.get()).await?) }
```

---

## 8. Performance — what “fast” means

Not “one shard per view.” Fast is:

- **Concurrent rendering** — sibling components, loop iterations, and nested depths `try_join!` together. No waterfalls; body must be side-effect free.
- **Memoization** — `#[memoize]` keyed by args + `ContextRead` tracking (`core/src/memoize/cache.rs`, `context/tracking.rs`). The same `load_user(cx, id)` from page + layout + table runs once. This is also the streaming cost control: re-renders of the page re-execute view building but not memoized I/O.
- **Preloading** — `include` over N+1 per-row `exec`. A list with 50 rows and 2 relations is 3 operations, not 101.
- **Boundaries** — ship only the table grid when search changes, not the sidebar + layout + headers (`DESIGN.md` hashing rule: child boundaries replaced by identity before parent hash).
- **Debounce + defer filters** — Filament's `searchDebounce(500ms)` / `deferFilters(true)` / `CanDeferLoading` placeholder apply. Argentum: `search.debounce_ms(300)` (client coalesce already + in-flight abort; add debounce for page refetch path) and `filters.defer(true)` (apply on button).
- **Pagination** — cursor, not offset, with PK tie-breaker. Extreme links off.

Budget v1: list render (25 rows, 2 includes, 1 count) < 40ms p50 on SQLite/Postgres local, TTFB dominated by the slowest `defer` region's skeleton, not the query.

---

## 9. Validation, errors & testing

- Validate in `Schema` (field rules), then in `Procedure`/`Shard` handler, then DB constraints. Return inline field errors (not toast-only). DB `#[unique]` violations cannot map to inline errors yet — toasty exposes no unique-violation predicate (see `EXTERNAL_GAPS.md`); pre-check uniqueness app-side until it lands.
- Router errors bubble via `Result` + `?` into layouts (`topcoat-router/docs/error.md`): `slot: Result` match on `NotFoundError` → `(StatusCode::NOT_FOUND) view!{...}`; otherwise `slot?`. Redirects via `Err(redirect("/..."))` — mid-stream redirects become swap instructions under streaming.
- Forms: `Action` handlers are `#[procedure]`-backed; errors render in modal's `Schema` without losing signal state.
- Testing: `CxTestBuilder` (`topcoat-core/src/context.rs`) for unit renders, `Page` golden tests, per-resource policy tests (`assert_can_list` / `assert_cannot_delete`). Bench via `benchmarks/` against `axum-maud`/`leptos`.

---

## 10. Security

- **Input escaping:** `like` patterns escape `%`/`_` from user input. Never interpolate raw `query` into `sql::query`.
- **Authorization:** Every resource defines `Policy` (`viewAny/view/create/update/delete`). Enforced in **both** page and shard/procedure. Default deny; `shouldSkipAuthorization` is opt-in and audited.
- **Tenancy:** `Resource::query(cx)` is the only place tenancy is applied. For `#[belongs_to] tenant_id`, the query is `filter(tenant_id().eq(tenant_id(cx)))` where `tenant_id(cx)` comes from `cx.with(Tenant(id))`. No `withoutGlobalScope` footgun.
- **Cookies/session:** `cookies(cx)` jar + `CookieStore<T>` (`topcoat/docs/cookie.md`, `session.md`) for remember-token, sliding expiration, origin-checked sessions (`layers::OriginPolicy`).

---

## 11. Roadmap

Each phase is shippable and benchable (`benchmarks/` vs `axum-maud`/`leptos`). No phase rewrites the previous one's resources.

### Phase 0 — Bootstrap (1–2 weeks)

- Workspace `argentum-core` + `argentum-macros` (`grammar` + `macro`), `Panel` + `Resource` trait + `Pages`, `Schema` layout primitives, `Db` glue (`db(cx)` + `#[memoize]` helpers), `coffee-shop`-style demo app with in-memory `User` model, `Router` + `module_router!` + `href!` + `tailwind` + `asset!`.
- CI: `cargo test` (sqlite), `topcoat fmt`, `clippy`. Bench harness stub.

### Phase 1 — Single-resource CRUD (exit: usable admin)

- `Table` (columns `searchable`/`sortable`, cursor pagination, `FilterBuilder`, debounced search via owned `#[shard]` + `boundary` skeleton) and `Schema` (6 fields: `TextInput`, `Select`, `Toggle`, `Textarea`, `DatePicker`, `Checkbox` + validation).
- `Create`/`Edit` pages (procedure-backed, inline errors) + `Action` (delete `requires_confirmation`, bulk delete) + notifications + auth shell + sidebar + empty/error states.
- Model: `User(id, name, email, role, active, created_at)` with `#[index]` on searchable columns.
- Exit: `cargo run` at `/admin/users` with search/sort/paginate/create/edit/delete, all policy-checked, no N+1 on list, bench reported. `key: &row.id` on every `for` row.

### Phase 2 — Relations & polish

- Relations: `HasMany`/`BelongsTo` via `include` in tables and `Select` lookups in forms. No `via` (SQL-only) in tables for v1.
- More fields/filters (`FileUpload`, `Repeater`, `SelectFilter`/`TernaryFilter`/`DateFilter`), grouping/summarizers (in-memory for v1), export (CSV via `Sse` / download route).
- Tenancy (`cx.with(Tenant)`) + per-tenant `canViewAny`.

### Phase 3 — Streaming & next runtime

- Adopt `defer`+`boundary` when landed (behind feature flag today; track `DESIGN_DELTA.md`: `ViewHandle<'_>` for named view args, blocking fallback outside `#[component]`/`#[page]`/`#[layout]`). Migrate `ArgentumTable` from single shard to page-level signal + boundary diff; keep `#[shard]` as opt-in for isolated heavy widgets. Add dev lint for unmemoized deferred loads.

### Phase 4 — Widgets & ecosystem

- Widgets (`StatsOverview`, `ChartWidget`), global search, `Infolist` entries, file/media, `topcoat-ui` themes, `embed_migrations!` history + `toasty-cli` standalone.

**Out of scope for v1:** `via` many-to-many in tables, DynamoDB-backed admin, `GROUP BY` aggregates beyond `count` (raw-SQL shim only), WASM admin, SPA mode.

---

## 12. Minimal example (current list slice)

The declarative Panel path is the shipped API. The complete runnable version,
including the Tailwind build contract and a typed table, lives in
`examples/showcase/src/app.rs`:

```rust
#[layout("/admin")]
async fn admin_layout(cx: &Cx, slot: Result) -> Result {
    Panel::layout_shell(cx, slot).await
}

fn router(db: toasty::Db) -> topcoat::router::Router {
    Panel::new("admin")
        .app_context(db)
        .assets(AssetBundle::load().expect("generated assets"))
        .shell_assets(tailwind::stylesheet!(), GEIST)
        .resource::<UserResource>()
        .build()
}

impl Resource for UserResource {
    fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {
        toasty::stmt::Query::<toasty::stmt::List<User>>::all()
    }

    fn table(cx: &Cx) -> Table<User> {
        Table::r#for(cx)
            .id(|user: &User| user.id.to_string())
            .columns(TextColumn::r#for(User::fields().name(), |user: &User| {
                user.name.clone()
            }).searchable().sortable())
            .paginate(25)
    }
}
```

---

## 13. Open questions (resolved later, not blocking Phase 0)

- `signal` ownership: `signal(cx, ||...)` vs ambient `signal(||...)` (`SIGNALS.md: Open questions: cx`). Argentum hides behind `ArgentumTable` prop so call sites don't care.
- Transport for signal refetch / boundary headers (`X-Topcoat-State` size limits at proxies). Same seam.
- Prewarm hint for `defer` (start memoized load during skeleton pass) and per-region flushing tradeoffs (`DESIGN-2.md: The trade`).
- `Table` column `key:` stability inside `for row in rows` — enforce `key: &row.id`.

---

## References

- Topcoat: `crates/topcoat/docs/getting_started.md`, `docs/context.md`, `docs/app_context.md`, `topcoat-core/macro/docs/memoize.md`, `topcoat-view/macro/docs/view.md`, `topcoat-view/macro/docs/component.md`, `topcoat/docs/runtime.md`, `topcoat-runtime/macro/docs/{shard,procedure,expr}.md`, `crates/topcoat-router/docs/{router,module_router,error,content/sitemap}.md`, `AGENTS.md`.
- Designs: `SIGNALS.md` (#335), `DESIGN.md` + `DESIGN-2.md` + `DESIGN_DELTA.md` (ssr-proposal #332, ssr-proposal-2).
- Toasty: `crates/toasty/docs/guide/{querying-records,filtering-with-expressions,sorting-limits-and-pagination,preloading-associations,schema-management}.md`, `docs/dev/roadmap.md`, `docs/dev/architecture/query-engine.md`, `examples/{quickstart-blog,forum-relationships,product-search}/*.rs`, `demos/coffee-shop/src/{app,models}.rs`.
- Filament (spirit, not API): `filament/packages/panels/src/Resources/Resource.php`, `packages/schemas/src/{Schema,Components/Component}.php`, `packages/forms/src/Components/Field.php`, `packages/tables/src/{Table,Columns/Column}.php`, `packages/actions/src/Action.php`, `packages/support/src/Concerns/EvaluatesClosures.php`, `CLAUDE.md`.
