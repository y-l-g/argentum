# Argentum

> **Filament for Rust** — a server-rendered admin toolkit on **Topcoat** (UI / reactivity) and **Toasty** (ORM).

Repo layout:

```
argentum/
  crates/argentum-core/    // Panel, Resource trait, Table/Schema types, navigation, policy, tenancy, auth
  crates/argentum-macros/  // #[derive(Resource)] (model/query only)
  crates/argentum-ui/      // vendored topcoat-ui primitives + owned composites
  examples/showcase/       // runnable admin: /admin/users + /admin/authors + /admin/posts
  benchmarks/              // Phase-2 harness + axum-maud/leptos baselines (detached workspaces)
  docs/adr/                // decisions; CONTEXT.md is the domain vocabulary
```

Shipped state, phases, and what comes next live in one place: [§10 Roadmap](#10-roadmap). Where this file and the original spec disagree, the **code and `docs/adr/` win**.

---

## 1. What it is / is not

**Is:**

- A server-side monolith. HTML is rendered on the server with `view!` / `#[component]`. No SPA, no client build, no WASM bundle.
- Type-safe: Toasty models → typed queries → typed tables and forms. Name a column that does not exist and it fails to compile.
- Fast by default: concurrent rendering + per-request memoization + explicit `include` preloading + streamed regions. No hidden N+1.
- A Topcoat citizen: layouts, `href!`, `Cx`, `#[memoize]`, `topcoat-ui`, `$(...)` expressions — not a parallel framework.

**Is not:**

- Not a Livewire port. No string `statePath`, no reflection DI, no `Macroable`, no Blade partials. Those patterns are PHP ergonomics; Rust has better ones.
- Not driver-agnostic at v1. The workspace exercises Toasty over **SQLite**; `via` many-to-many and DynamoDB-backed admins are explicitly out of scope for tables.
- Not a client framework. `$(...)` and signals are a tiny JS vocabulary. Anything that needs the DB is a server render.

---

## 2. Principles

These are invariants. Code that violates them is a bug.

1. **Pure renders.** Pages, layouts, and components are side-effect free and deterministic. No `HashMap` iteration in a streamed region, no `Utc::now()` inside one, no random ID per render. Required for concurrent rendering and streaming re-renders.
2. **One truth for data.** Components query Toasty directly. No REST layer between UI and DB. `Db` lives in `app_context` and is cloned per request (`argentum_core::db::db(cx).clone()` → `&mut db` for `exec`).
3. **Server inputs are untrusted.** Every POST handler and signal value comes from the client — including values read on the server via `get()`/`read()`, which the client chooses like a shard argument. `can_*` checks run **inside** every handler — layout guards do not cover them.
4. **Explicit is fast.** `include` for relations, `#[index]` for filter columns, `#[memoize]` for shared loads, streamed regions around skeletons. No magic preloading, no implicit scans.
5. **Composable `Cx`, not middleware.** Tenant, locale, auth are `cx.with(Tenant(id))` scoped values and `fn require_*(cx: &Cx)` helpers. No Tower layer for business logic.

---

## 3. Architecture

Every app depends on **`argentum-core`** (which in turn depends on **`argentum-macros`** and **`argentum-ui`**). UI chrome lives in **`argentum-ui`** (ADR-0006/0007): its *primitives* are a verbatim mirror of `topcoat-ui-registry`, synced by `cargo xtask sync-topcoat-ui` and never hand-edited; its *composites* (Sidebar, Page, CodeBlock) are owned Argentum components.

Topcoat stack assumptions (Topcoat 0.8 / Toasty 0.10, both tracking `main`, pins in `Cargo.lock` — bump deliberately with `cargo update -p topcoat` / `cargo update -p toasty`): `view!` / `#[component]` (concurrent), `#[page]` / `#[layout]` / `discover` + `href!`, `Cx` with `cx.with(...)` + `#[memoize]`, `cookie`/`session`, `asset!`, `tailwind`. `Panel::build` mounts the browser-runtime routes (`RouterBuilderRuntimeExt::runtime()`, required by `runtime::script`). Streaming SSR via `suspense` is **adopted** — the resource list streams its rows behind a skeleton — and rerun morphing is **adopted** (Topcoat #392: shard/page re-runs morph in place, focus survives); `Signal<T>` shard params landed (Topcoat #393) and the keystroke-live table shard landed behind `Table::live_search` (#104); Argentum keeps working on today's runtime (see §7). `TowerRoute::any` has no Argentum use (we mount no tower services).

Upstream gaps — missing or unstable Toasty/Topcoat APIs that force Argentum workarounds — are tracked as GitHub issues labelled [`upstream`](https://github.com/y-l-g/argentum/labels/upstream), one per API, each recording *Where / Today / Why fragile / Clean upstream API / Argentum plan*. Reach into upstream internals (`toasty_core`, `topcoat::view::internal`) only with such an issue open; retire the issue when the upstream fix lands and delete the workaround it justified.

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

`Panel` owns the `Router`, registers `Db` and declared resources in `app_context`, registers each resource list at `/{prefix}/{slug}`, and redirects the panel root to the first resource. An app registers the shell with one layout handler: `Panel::layout_shell(cx, slot).await`. Custom pages add a typed `NavigationItem::from_href` through `Panel::navigation(..)`. `.brand(..)` / `.dark_mode(bool)` are opt-in shell seams (header brand + `dark` class + theme toggle); the showcase router leaves them unset and the shell falls back to the `"Admin"` title.

### 4.2 `Resource`

The mapping from a Toasty model to its admin UI. One resource = one model = a set of pages (list / create / edit). Heavily inspired by Filament's `Resource.php` but typed.

```rust
pub trait Resource: Sized + Send + Sync + 'static {
    type Model: toasty::schema::Model + Send + Sync + 'static;

    // Authorization — default-deny, checked in page *and* POST handlers.
    fn can_view_any(_cx: &Cx) -> bool { false }
    fn can_view(_cx: &Cx, _record: &Self::Model) -> bool { false }
    fn can_create(_cx: &Cx) -> bool { false }
    fn can_update(_cx: &Cx, _record: &Self::Model) -> bool { false }
    fn can_delete(_cx: &Cx, _record: &Self::Model) -> bool { false }

    fn slug() -> String { /* UserResource -> users, BlogPostResource -> blog-posts */ }
    fn navigation_label() -> String { /* plural model name */ }
    fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Self::Model>> {
        toasty::stmt::Query::<toasty::stmt::List<Self::Model>>::all()
    }
    fn table(_cx: &Cx) -> Table<Self::Model> { Table::new() } // default: empty, not renderable until columns + id
    fn form(_cx: &Cx) -> Schema { Schema::empty() }
    fn pages() -> Pages<Self> { Pages::crud() } // Phase 1 stub: Panel does not consume this yet (#106)
    fn navigation() -> NavigationItem { NavigationItem::from_resource::<Self>() }

    // Record operations driven by the create/edit/delete POST handlers.
    fn create_record(_cx: &Cx, _values: HashMap<String, String>, _ex: &mut dyn toasty::Executor) -> impl Future<Output = Result<()>> + Send;
    fn update_record(_cx: &Cx, _record: Self::Model, _values: HashMap<String, String>, _ex: &mut dyn toasty::Executor) -> impl Future<Output = Result<()>> + Send;
    fn delete_record(_cx: &Cx, _record: Self::Model, _ex: &mut dyn toasty::Executor) -> impl Future<Output = Result<()>> + Send;
    fn bulk_delete_records(_cx: &Cx, _records: Vec<Self::Model>, _ex: &mut dyn toasty::Executor) -> impl Future<Output = Result<()>> + Send;
    fn hydrate_form_values(_record: &Self::Model) -> HashMap<String, String>;
}
```

*Derive macro (model/query only):*

```rust
#[derive(Resource)]
#[resource(model = User)]
struct UserResource;

#[derive(Resource)]
#[resource(model = User, query = only_ada)]
struct ScopedUserResource;
```

The derive implements `Resource` for the annotated type with the given `Model` and optional `query` override. It does **not** cover `table`/`form` — those are hand-written; lenses are `User::fields().email()`.

`Panel::resource::<UserResource>()` registers the resource list at `/admin/users` (slug derived from the resource type) and derives the matching sidebar item. `Resource::query` is the single tenancy seam: `fn query(cx: &Cx) -> Query<List<User>> { Query::<List<User>>::all().filter(User::fields().tenant_id().eq(tenant_id(cx))) }` via `cx.with(Tenant(id))`.

### 4.3 `Schema` (forms)

Layout primitives plus typed fields bound via lenses — no string paths.

```rust
Schema::new((
    Section::new("Account").schema((
        TextInput::r#for(User::fields().email()).required().email().unique(),
        Select::r#for(User::fields().role()).options(Role::variants()),
    )),
    Grid::new(2).schema((
        TextInput::r#for(User::fields().name()).required(),
        FileUpload::r#for(Post::fields().image_path()).required(),
    )),
))
```

- Every field takes a **typed lens** `FieldPath<Model, T>`, not a string. The lens knows `#[column]` renames.
- Layout primitives: `Section`, `Group`, `Grid(12)`, `Tabs`, `Wizard` — plus display `Text`.
- Fields: `TextInput` (`required`/`email`/`unique`), `Select` (`options`/`options_with_labels`/`relationship`), `FileUpload`, `Repeater`. There is no `Toggle`, `exists`, or `regex` builder — relationship existence is checked app-side in the record handlers.
- `FileUpload` stores the sanitized basename as the `String` path (bytes not persisted in v1, no `value` on `type=file`); forms containing one emit `enctype="multipart/form-data"`. Multipart streams field-by-field with constant memory (file bytes drained and discarded); form bodies are capped at 10 MiB (413) and bare multipart without a boundary is a 400 (GH #90). Empty file submits preserve the stored path; `clear_<field>=1` opts back into clearing.
- `Repeater` is a single-entry group; its label-keyed `required` error renders inline.
- State is the Toasty model itself (or a `Create`/`Update` projection). Hydration is `model -> Schema`, dehydration is `Schema -> Update` / `Create` + validation.
- Validation is **app-level** (`required`, `email`, `unique` pre-check). Toasty column constraints (`#[unique]`, type) are DB-level; toasty exposes no unique-violation error predicate yet (upstream gap #117), so DB violations cannot map to field errors today — uniqueness is checked app-side, which races under concurrent writes.

### 4.4 `Table`

Composition, not a 30-trait God object. `Table<M>` is a value describing the list view; Argentum renders it. As shipped today (see `examples/showcase/src/app.rs`):

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

Columns/filters declare **how to query**, not just how to render: `searchable()` marks a column for portable `starts_with` search, `sortable()` maps to `order_by` (toasty appends PK tie-breakers internally for deterministic cursors), and the URL is the one truth — `?q=`, `?sort=`, `?dir=`, `?after=`, `?before=`, `?filters=`, `?group_by=` parsed once into `TableState`.

- **Filters:** the `Filter` enum (`SelectFilter`/`TernaryFilter`/`DateFilter`/`VariantFilter`) over `IntoFilters` tuples. `Table::filter_expr` ANDs the active `?filters=` expressions into the loader. `VariantFilter` holds prebuilt `(label, Expr<bool>)` options (e.g. `is_variant()` predicates) for embedded-enum fields. Every declared filter renders a typed control composed into the single `?filters=` param (free-text input stays as fallback). Unknown keys and rejected values never fail silently: the list renders a `role=alert` banner (`Table::unapplied_filters`) while export refuses with 400.
- **Bulk selection** is a leading checkbox column with select-all whose JS joins keys into the single `ids` transport (text field stays as fallback).
- **Grouping/export:** in-memory named `group_by` + `count` summarizer + `Table::to_csv()` (RFC4180, OWASP formula-defused) via `GET /admin/{slug}/export` (`text/csv; charset=utf-8` + `Content-Disposition`), reusing `Resource::query` + filters/sort, capped at 10k rows (413 past the cap, `?bom=1` opts into an Excel BOM). Unknown `?group_by=` values render no headers and drop from nav links. Grouping is page-local by design (Toasty has no `GROUP BY` yet — upstream gap #118); export renders the ungrouped full filtered set.
- **Rendering:** `Table` is a boundary by default (`data-boundary="table"` wrapper) with an eager-render `defer` demo hook; the streamed list uses the skeleton as its `suspense` fallback, and failed loads render the branded `ErrorState` in-region.

### 4.5 Deletes, notifications, policy

There is no `Action` type. Deletes run through panel POST routes driving the `Resource` record fns:

- `POST {list}/{id}/delete` → `delete_record`, with confirmation UI and a per-row `can_delete` re-check inside the handler.
- `POST {list}/bulk-delete` → `bulk_delete_records`, all-or-nothing: every id is re-fetched via `Resource::query` and policy-checked before anything is deleted.
- Create/edit POSTs validate inline, check `can_create`/`can_update`, then call `create_record`/`update_record`.

`Notification` (`success`/`error`/`info`) travels via `Set-Cookie` (`argentum_notification`) with a `?notification=` fallback and renders in a shell-level stack (`fixed top-4 right-4`) that survives table swaps. Policy is `Resource::can_*`, default-deny, enforced in both page and POST handlers; the standalone `Policy<R>` trait (`AllowAll`/`DenyAll`) exists as a helper.

### 4.6 Authentication

`Panel` is gated by default and fails closed (ADR-0013, spec #127): an unauthenticated panel page redirects to `{prefix}/login` with a validated same-origin `next`, runtime endpoints answer 401, and a user without `can_access_panel` answers 403.

- **Zero-config default.** Register the shipped models (`toasty::models!(…, argentum_core::auth::AdminUser, argentum_core::auth::AuthSession)`), seed an `AdminUser` (`argentum_core::auth::hash_password("…")` stores Argon2id PHC strings), and log in through `GET`/`POST /admin/login`; `POST {prefix}/logout` revokes. Sessions are server-side `AuthSession` rows keyed by token hash, seven-day fixed lifetime, rotated on login, revocable per user with `auth::revoke_sessions_for_user(cx, id)`.
- **One override seam.** An app with its own user table implements `Authenticator` (`verify` + `find_by_id`) and passes `Panel::auth(Auth::custom(MyAuth))`; it still registers `AuthSession`, which owns session storage. Resolution yields one erased `CurrentUser { id, login, display_name, tenant_id, can_access_panel }` in request `Cx`; read it with `current_user(cx)` / `require_authenticated(cx)`.
- **Explicit opt-out.** `Panel::auth(Auth::disabled())` serves the panel without a gate — greppable, never implicit.
- **Tenancy from the login.** The user's optional `tenant_id` is injected as `Tenant` into the request, so `/admin/authors` and `/admin/posts` scope to the logged-in admin; the `x-tenant-id` header is never trusted (GH #131). A server-set `Tenant` request extension overrides deliberately.
- **Login page.** `Panel::login_hint("Demo: admin@example.com / password")` renders a muted line under the shipped form for demos; brand and dark mode carry over from the panel.

The showcase proves the default (`examples/showcase/tests/auth_check.rs`); `crates/argentum-core/tests/auth_override.rs` proves the custom model path end to end.

---

## 5. Pages, routing, navigation

- **Routes** (all derived from the panel prefix + resource slug): `GET /admin/users` (list), `GET`+`POST /admin/users/create`, `GET`+`POST /admin/users/{id}/edit`, `POST /admin/users/{id}/delete`, `POST /admin/users/bulk-delete`, `GET /admin/users/export`. Auth mounts `GET`+`POST {prefix}/login` and `POST {prefix}/logout` (ADR-0013). Additional `#[page]` handlers are discovered normally. `GET /admin` redirects to the first declared resource.
- **Layouts:** an app's `#[layout("/admin")]` handler delegates to `Panel::layout_shell(cx, slot)`. The shell owns the complete document, sidebar, runtime scripts, and links to the app-provided `tailwind::stylesheet!()` and `fontsource_font!(.., host: Asset)` handles.
- **Navigation:** `Panel::resource` derives one item per resource with prefix-aware URLs (`from_resource_with_prefix`; `from_resource` is the `"/admin"` shorthand). `Panel::navigation(NavigationItem::from_href(..))` adds typed links for custom pages. Resource items use exact paths plus slash-boundary subpages for active state.
- **Errors & redirects:** layouts receive the page as `slot: Slot<'_>` (a lazy `Child`); a page error propagates through the slot when the document resolves, and the router maps it onto its HTTP response (`error_boundary` around the slot can replace this with a branded error page). Failed table loads are caught in-region and render `argentum_ui::error_state` inside the streamed region. Note: once the first content streamed, the status line is fixed — an error in a streamed region truncates the body (redirects degrade to `window.location.replace`), so streamed regions own their failure rendering.

---

## 6. Data layer — Toasty

### 6.1 The seam

```rust
fn db(cx: &Cx) -> Db { app_context::<Db>(cx).clone() }

// in a page/POST handler:
let mut db = db(cx);
let rows = User::all().exec(&mut db).await?;
```

`Db` is `Arc`-pooled. `exec` needs `&mut Db` — clone per request is cheap.

### 6.2 How tables and searches query

There is **no** `User::find().where(User::name.contains(&q)).all(cx.db())`. That API does not exist. The real shape:

```rust
// exact:
User::filter(User::fields().email().eq("alice@example.com"))
// prefix (portable):
User::filter(User::fields().name().starts_with(q.clone()))
```

- `starts_with` is the only portable string predicate. `like` is SQL-only (escape `%`/`_` from user input first), `ilike` is Postgres-only. `contains` on `String` does not exist.
- Global search across `searchable()` columns is `Expr::or` over each column's predicate.
- Toasty has no conditional-query builder yet, so loaders filter imperatively from `Resource::query` (`Table::filter_expr` ANDs the active filter expressions).

### 6.3 Sorting & pagination

- **Sorting:** `order_by(col.asc()/desc())`, tuples or chained calls; each `sortable()` column maps to one.
- **Pagination:** cursor pagination, **requires** `order_by`. `paginate(per_page)` returns a `Page<M>` (upper bound, `has_next()` not `len`); toasty appends the PK tie-breaker internally for determinism. `limit`/`offset` exists but `offset` requires `limit` and is slow at large offset — prefer cursor.

### 6.4 Relations & N+1

Use `Deferred<Vec<Post>>` / `Deferred<Author>` and preload with **one round-trip**:

```rust
User::all().include(User::fields().posts()).exec(&mut db).await?;
// then `user.posts.get().len()` — no await
```

Eager `Vec<T>` is for tiny relations only — eager cycles are a compile-time schema-build error. `has_many(via = …)` many-to-many is **SQL-only, read-only** — out of scope for v1 tables; use the join-model query when DynamoDB compatibility is desired. Computed columns (`TextColumn::computed`) and `Select::relationship` reuse `Resource::query` so tenancy is preserved; relation cells must handle the unloaded case (the showcase renders `"(unloaded)"` and `debug_assert!`s that `include` ran, GH #101).

### 6.5 Aggregates, schema & migrations

Toasty has `count()` but no `GROUP BY / HAVING / SUM / DISTINCT` yet — grouping stays in-memory over the loaded page (`count` only, labelled page-local) and raw SQL is not used in table code. `Db::builder().models(toasty::models!(crate::*)).connect(url).await?; db.push_schema().await` for POC; prod uses `embed_migrations!()` + `history.toml` + `snapshots/*.toml` via `toasty-cli`.

---

## 7. Reactivity — today's API and the seam that survives

Today on `main` (topcoat 0.8):

```rust
let query = signal(cx, String::new); // page-owned; hoists identity for the client
<input :value=$(query.get()) @input=$(|e: Event| query.set(e.target.value))>
```

- `signal(cx, init)` is an ordinary Rust function returning an owned cheap-to-clone value. It must run inside a page/layout/component body (hand-registered `PageFn`s wrap their body in `HoistView`, exactly what `#[page]` generates). Identity comes from the creating body plus the call site, so a body that repeats (for loop) needs a `key`.
- Reads in `$(...)` re-run in JS with no server round-trip. Reads in plain Rust via `get()`/`read()` are **tracked** — they emit a dependency marker so a browser change re-runs the page (or the innermost enclosing shard) via the runtime routes, and the result is **morphed** into place (Topcoat #392): elements that still exist update in place so focus, scroll position, and what the user is typing survive; give reorderable list items a stable `id` so the morph follows each item. `get_untracked()`/`read_untracked()` opt out. Every server-read value is untrusted user input.
- A shard can also take a signal from its caller through a parameter typed `Signal<T>`, passed as `$(signal)` (Topcoat #393). The handle does not change when its value does, so whether a change re-renders the shard depends on how the shard body reads it — the seam for passing table state without forcing re-renders (see #104).
- Guards on page/layout **do not run** on shard requests — a shard must authorize itself. Argentum's `table_search` shard does (`requires_tenant` + `can_view_any`, row scoping via `Resource::query`); deletes/creates/edits stay POST `PageFn` handlers, and the hand-registered pages adapt fallible async bodies with an internal view adapter mirroring `#[page]` (upstream gap #123).
- **Adopted:** `suspense(fallback, child)` for streaming skeletons — first content ships the shell + skeleton, the loaded content swaps in via `<template data-topcoat-swap>` markers, no client library. (Upstream also ships `live!`/`emit!`; Argentum does not use them.) `resource_list` streams rows this way; `Table::render_skeleton` is the shared fallback.
- **Designs, not mechanisms:** the keystroke-live table shard (ticket #104, shipped as one slug-dispatched `table_search` shard fanning out through the panel's per-resource registry — inventory only discovers concrete fns, so a generic shard is undiscoverable — with the swapped region morphing per #392 and stable morph `id`s on rows). `Table::render_with_state` + `TableState::from_live_args` are the kept seam; `Table::live_search(true)` opts a table in, and the `?q=` GET toolbar stays as the no-JS fallback.

**Argentum's contract (works on `main`, migrates without rewriting resources):** tables and slow cards are **streamed regions**; filter/search/sort/page state is page-owned URL state; shared loads are **`#[memoize]`d** so concurrent rendering and fan-out dedup are free. Relationship option loads ship that way today; the streamed table loader does not yet (a dev lint for unmemoized deferred loads is on the Now list).

---

## 8. Performance, security & testing

Fast is: **concurrent rendering** (siblings `try_join!`, no waterfalls, side-effect-free bodies), **memoization where declared** (`#[memoize]` keyed by args — relationship option loads today; page re-renders rebuild views, not I/O), **preloading** (`include`: 50 rows + 2 relations = 3 operations, not 101), **streamed regions** (skeleton first, grid swaps in). Client changes coalesce and in-flight requests abort; explicit debounce/defer-filter controls are future work.

Budget: list render (50 rows, 2 includes) < 40ms p50 on local SQLite, TTFB dominated by the skeleton — rows arrive as a streaming swap. Harness: `benchmarks/` vs `axum-maud`/`leptos` stubs (`cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench`, `./benchmarks/scripts/bench.sh`, `verify_parity.sh`).

Security: `like` patterns escape `%`/`_`; never interpolate raw input into raw SQL. Every mutation runs in a framework-owned transaction (GH #84): handlers open the tx, load + policy-check records on that snapshot, and pass the checked records into the `Resource` record fns — no silent re-loads (GH #86 TOCTOU). Every resource enforces `can_*` in page **and** POST handler (default deny); edit GET and POST both require `can_view` + `can_update`, export drops rows failing per-row `can_view`, while the list checks only `can_view_any` by design (GH #86: in-memory predicates can't paginate honestly — list-level row scoping belongs in `Resource::query`). Tenancy applies only in `Resource::query` (`tenant_id(cx)` from the logged-in user's `Tenant`, or a server-set request extension; no header, GH #131). The panel is gated by default (ADR-0013): the auth layer resolves the session into `Cx`, pages redirect, runtime endpoints answer 401, passwords verify Argon2id with a dummy hash for unknown emails, and login failures render one generic message; handlers call `require_authenticated` as defense in depth. All POSTs require a double-submit `csrf_token` (GH #99); `confirm=1` is a UX step, not a boundary. Cookies/sessions via Topcoat's `cookie`/`session` + origin-checked sessions.

Testing: `CxTestBuilder` for unit renders, per-resource policy tests, showcase integration tests (`examples/showcase/tests/`: admin, create/edit/delete/bulk, relations, filters, tenancy, file+repeater, group+export, auth).

---

## 9. Validation & errors

Validate in `Schema` (field rules), then in the POST handler, then DB constraints. Return inline field errors (not toast-only). Absent keys validate as `""`, so updates must only write present keys; create/edit POSTs reject unknown keys with 400 via `Schema::unknown_keys` (GH #89 — `role`/`tenant_id` smuggling fails closed at the framework layer, `csrf_token` is the only handler key exempted). DB `#[unique]` violations cannot map to inline errors yet — toasty exposes no unique-violation predicate (upstream gap #117); uniqueness is pre-checked app-side until it lands. Router errors bubble via `Result` + `?` into the router's error→status mapping; a layout can wrap its `Slot<'_>` in `error_boundary` to brand them. Redirects via `Err(redirect("/..."))` — before first content they are `Location` responses; mid-stream they degrade to a `window.location.replace` script.

---

## 10. Roadmap

### Shipped — Phase 1: single-resource CRUD (spec #57, tickets #58–#62, ADR-0010)

`Table` with typed row keys, `searchable`/`sortable` columns, cursor pagination, `?q=` toolbar and streamed grid; `Schema` hydration via typed lenses + inline validation; create/edit pages; row + bulk delete (policy-checked, all-or-nothing bulk); `Notification` surviving table swaps; `can_*` default-deny policy; auth shell + sidebar + empty/error states. Showcase at `/admin/users` (see `examples/showcase/tests/{admin,create_check,edit_check,delete_check,bulk_check}.rs`).

### Shipped — Phase 2: relations & polish (spec #63, tickets #64–#71, ADR-0011/0012)

`Post` with `BelongsTo author` + `HasMany comments` via `include` (one round-trip, `TextColumn::computed` + `Select::relationship` reusing `Resource::query`); `FileUpload`/`Repeater` in `Section`/`Grid`; `SelectFilter`/`TernaryFilter`/`DateFilter`/`VariantFilter` via `Filter` + `TableState ?filters=`; in-memory `group_by` + `count` + `to_csv()` with `GET /admin/{slug}/export`; tenancy (`Tenant` + per-tenant policy); `Panel::brand` + `Panel::dark_mode`; `benchmarks/` Phase-2 budget. Showcase at `/admin/posts` + `/admin/authors` (see `relation_check`, `filter_check`, `tenancy_check`, `file_repeater_check`, `group_export_check` tests).

### Shipped — post-Phase-2 polish (#73, #74, #77, #78, #79, #76, #104)

Multipart `FileUpload` (filename-as-path, no `value` on `type=file`), single-entry `Repeater` docs + inline required error, `VariantFilter`, honest table chrome (checkbox bulk column, typed filter widgets, dummy live-search shard removed in favor of the GET toolbar), in-region `ErrorState` for failed streamed loads, PK tie-breaker delegated to toasty, keystroke-live table search behind `Table::live_search` on the `render_with_state` + `from_live_args` seam with stable morph `id`s on reorderable rows.

### Shipped — authentication (spec #127, tickets #128–#132, ADR-0013)

Default-on panel gate: shipped `AdminUser` + `AuthSession` models, Argon2id `PasswordAuth` with one generic failure, server-side seven-day sessions (rotated on login, revoked on logout, revocable per user), standalone login page with brand/dark mode + `login_hint`, the `Authenticator` override seam proven end to end (`crates/argentum-core/tests/auth_override.rs`), and the logged-in user's tenant replacing the `x-tenant-id` header. Showcase proof: `examples/showcase/tests/auth_check.rs`.

### Now

Dev lint for unmemoized deferred loads, prewarm hint for `defer`, per-region flush tradeoffs.

### Next

Widgets (`StatsOverview`, chart), global search, infolist entries, file/media assets, themes beyond brand/dark-mode tokens, `embed_migrations!` history + `toasty-cli` standalone.

**Out of scope for v1:** `via` many-to-many in tables, DynamoDB-backed admin, `GROUP BY` aggregates beyond `count` (raw-SQL shim only), WASM admin, SPA mode.

(The old tracking issue #38 is closed; its remaining future slices are the Now/Next lists above. Open work is tracked per-topic in #88 (unique-check race/scope) and #91 (relationship Select), plus Renovate's `#82`.)

---

## 11. Minimal example (mirrors `examples/showcase/src/app.rs`)

```rust
#[layout("/admin")]
async fn admin_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    Panel::layout_shell(cx, slot).await
}

fn router(db: toasty::Db) -> topcoat::router::Router {
    Panel::new("admin")
        .app_context(db)
        .assets(AssetBundle::load().expect("generated assets"))
        .shell_assets(tailwind::stylesheet!(), GEIST)
        .resource::<UserResource>()
        .resource::<AuthorResource>()
        .resource::<PostResource>()
        .build()
}

impl Resource for PostResource {
    type Model = Post;
    fn query(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Post>> {
        let mut q = toasty::stmt::Query::<toasty::stmt::List<Post>>::all();
        if let Some(tid) = tenant_id(cx) { q = q.filter(Post::fields().tenant_id().eq(tid)); }
        let inc_author: toasty::stmt::Include<Post, Author> = Post::fields().author().into();
        let inc_comments: toasty::stmt::Include<Post, toasty::stmt::List<Comment>> =
            Post::fields().comments().into();
        q.include(inc_author).include(inc_comments) // one round-trip, no N+1
    }
    fn table(cx: &Cx) -> Table<Post> {
        Table::r#for(cx)
            .id(|p: &Post| p.id.to_string())
            .columns((
                TextColumn::r#for(Post::fields().title(), |p: &Post| p.title.clone()).searchable().sortable(),
                // `include`d relations still guard the unloaded case:
                TextColumn::computed("Author", |p: &Post| {
                    if p.author.is_unloaded() { "(unloaded)".to_string() } else { p.author.get().name.clone() }
                }),
                TextColumn::computed("Comments", |p: &Post| {
                    if p.comments.is_unloaded() { "(unloaded)".to_string() } else { p.comments.get().len().to_string() }
                }),
            ))
            .filters((
                SelectFilter::r#for(Post::fields().status(), vec!["draft".into(), "published".into()]),
                TernaryFilter::r#for(Post::fields().featured()),
                DateFilter::r#for(Post::fields().created_at()),
            ))
            .group_by("status", |p: &Post| p.status.clone())
            .paginate(2)
    }
    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            Section::new("Post Details").schema((
                TextInput::r#for(Post::fields().title()).required(),
                Select::r#for(Post::fields().author_id())
                    .relationship::<AuthorResource>(AuthorResource::query, |a: &Author| a.name.clone())
                    .required()
                    .label("Author"),
            )),
            Grid::new(2).schema((
                FileUpload::r#for(Post::fields().image_path()).required(),
                Repeater::new("Tags").schema(TextInput::r#for(Post::fields().tags()).required().label("Tag")),
            )),
        ))
    }
}
```

---

## 12. Open questions

- Transport for re-runs: runtime page/shard routes landed and results morph in place (#392 — focus/scroll/typing survive, stable `id`s pin reorderable items). Shard `Signal<T>` params landed (#393); the live table search shard landed behind `Table::live_search` (ticket #104). Same seam.
- Prewarm hint for `defer` (start memoized load during skeleton pass) and per-region flushing tradeoffs.
- `Table` column `key:` stability inside `for row in rows` — enforce `key:` from the row key, never the loop index.

---

## References

- This repo: `CONTEXT.md` (vocabulary), `docs/adr/` (decisions), the `upstream`-labelled issues (Toasty/Topcoat gaps), `benchmarks/README.md`, `examples/showcase/` (runnable truth).
- Toasty guide: querying records, filtering with expressions, sorting/limits/pagination, preloading associations, schema management; `toasty-cli` for migrations.
- Topcoat docs: `runtime`, `memoize`, `view`/`component`, `shard`/`procedure`/`expr`, router (`router`, `module_router`, `error`, sitemaps), cookie/session, `functions_not_middlewares`.
- Filament PHP (spirit, not API): `panels/Resources/Resource.php`, `schemas/Schema.php`, `tables/Table.php`, `actions/Action.php`.
