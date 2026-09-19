# Argentum

> **Admin toolkit for Rust**, server-rendered on **Topcoat** (UI and reactivity) and **Toasty** (ORM). Filament-style Panel plus Resource, tables, and forms, with no SPA build step.

Repo layout:

```
argentum/
  crates/argentum-core/    # Panel, Resource trait, Table and Schema types, auth, tenancy
  crates/argentum-macros/  # derive(Resource) for model and query only
  crates/argentum-ui/      # Topcoat primitives plus owned composites (Page, Toast, Theme, ErrorState, CodeBlock, BoundInput)
  examples/showcase/       # runnable admin: /admin/users, /admin/authors, /admin/posts
  benchmarks/              # perf harness plus axum-maud and leptos baselines
  docs/adr/                # design notes; CONTEXT.md is the vocabulary
```

Run the showcase:

```sh
cargo run -p showcase
# open http://localhost:3000/admin/users
```

---

## 1. What it is / is not

Is:

- Server-rendered HTML with `view!` and `#[component]`. No SPA, no WASM bundle.
- Typed end to end: Toasty model to query to table and form. A bad column name fails to compile.
- Fast by default: concurrent renders, preloaded relations, cursor pagination.
- A Topcoat app: layouts, `href!`, `Cx`, `#[memoize]`, small `$(...)` expressions.

Is not:

- Not a Livewire port. No string state paths, no reflection DI, no Blade partials.
- Not driver-agnostic in v1. The workspace targets Toasty over SQLite.
- Not a client framework. Anything that needs the DB renders on the server.

---

## 2. Quick start

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
`create_record` / `update_record` fns (see §4).

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
        // `AdminUser` + `AuthSession` in `toasty::models!` instead (see §7).
        .auth(Auth::disabled())
        .build().expect("panel builds")
}
```

See `examples/showcase/src/app.rs` for the full version with forms, filters, and tenancy.

---

## 3. Panel and routing

`Panel` owns the router, the `Db` in app context, and the shell layout. Registering a resource adds its routes and its sidebar item.

Routes for a resource with slug `users` under prefix `admin`:

- `GET /admin/users` : list
- `GET + POST /admin/users/create` : create
- `GET + POST /admin/users/{id}/edit` : edit
- `POST /admin/users/{id}/delete` : delete with confirm step
- `POST /admin/users/bulk-delete` : bulk delete
- `GET /admin/users/export` : CSV export
- `GET /admin` redirects to the first resource

Useful panel options:

```rust
Panel::new("admin")
    .brand(Brand::new("Acme"))
    .dark_mode(true)
    .navigation(NavigationItem::from_href("Docs", href!("/admin/docs"), "/admin/docs"))
    .login_hint("Demo: admin@example.com / password")
```

`brand` sets the header and sidebar name. `dark_mode` sets the initial theme; the toggle is always rendered and the stored choice wins later.

---

## 4. Resource

One resource maps one Toasty model to its admin UI:

```rust
pub trait Resource: Sized + Send + Sync + 'static {
    type Model: toasty::schema::Model + Send + Sync + 'static;
    fn query(_cx: &Cx) -> Query<List<Self::Model>>; // default: Query::all()
    fn table(_cx: &Cx) -> Table<Self::Model>;       // default: Table::new(), empty until columns + id
    fn form(_cx: &Cx) -> Schema;                    // default: Schema::empty()
    // plus can_* policy fns (default deny), slug/navigation/requires_tenant
    // defaults, and create/update/delete record fns
}
```

What to know:

- `slug()` and `navigation_label()` have working defaults. Override only to rename.
- `navigation()` curates this resource's sidebar entry: override it to order or group the entry, e.g.
  `NavigationItem::for_resource::<Self>().sorted(-1)` (lower `order` renders first, ties keep
  declaration order). The URL is the Panel's call: `for_resource` names none, so the panel that mounts
  the resource resolves it to `{prefix}/{slug}`, and a resource never links at `/admin` on a panel
  mounted elsewhere. A URL you spell out instead (`NavigationItem::at(..)`, `from_href`) is kept
  verbatim — use it to link somewhere other than the resource's list page.
- `query()` is the scoping seam. All list, export, and relation loads use it. Put tenancy here.
- `table()` and `form()` are hand-written. The derive only fills in `Model` and an optional `query`:

```rust
#[derive(Resource)]
#[resource(model = User)]
struct UserResource;
```

- Record fns (`create_record`, `update_record`, `delete_record`, `bulk_delete_records`) do the writes. Handlers load records, check policy, then call them in a transaction.
- For edit forms, `hydrate_form_values` maps a record to initial field values.

Tenancy pattern:

```rust
fn query(cx: &Cx) -> Query<List<Post>> {
    let mut q = Query::<List<Post>>::all();
    if let Some(tid) = tenant_id(cx) {
        q = q.filter(Post::fields().tenant_id().eq(tid));
    }
    q
}
```

---

## 5. Tables

Minimal table:

```rust
Table::r#for(cx)
    .id(|u: &User| u.id.to_string())
    .columns((
        TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone())
            .searchable()
            .sortable(),
        TextColumn::computed("Status", |u: &User| {
            if u.active { "Active".into() } else { "Inactive".into() }
        }),
    ))
    .paginate(20)
```

Notes:

- `.id(...)` is required. It keys rows for selection and live updates. Never use a loop index.
- `searchable()` searches with `?q=`: an escaped substring match (`like_with_escape`, OR across searchable columns), so a term containing `%` or `_` matches those characters literally. `LIKE` is ASCII-case-insensitive on SQLite and case-sensitive on PostgreSQL. `sortable()` sorts with `?sort=` and `?dir=`. Both work without JS.
- The URL is the state: `?q=`, `?sort=`, `?dir=`, `?after=`, `?before=`, `?filters=`, `?group_by=` parse into `TableState`. Pagination is cursor based; Toasty appends the PK tie-breaker internally so cursors stay deterministic.
- Computed columns render only. They do not affect search or sort.

Filters:

```rust
.filters((
    SelectFilter::r#for(Post::fields().status(), vec!["draft".into(), "published".into()]),
    TernaryFilter::r#for(Post::fields().featured()),
    DateFilter::r#for(Post::fields().created_at()),
))
```

Active filters travel in `?filters=` and combine with AND. Unknown keys and rejected values never fail silently: the list renders a `role=alert` banner (`Table::unapplied_filters`) while export refuses with 400.

Grouping and export:

```rust
.group_by("status", |p: &Post| p.status.clone())
```

- Grouping is page-local with a row count per group. Toasty has no `GROUP BY` yet, so grouping never claims full-table totals. Unknown `?group_by=` values render no headers and drop from nav links.
- `GET /admin/{slug}/export` returns the filtered set as CSV (`text/csv; charset=utf-8` + `Content-Disposition`, RFC4180 with OWASP formula-defusing), reusing the same `query`, filters, and sort. Capped at 10k viewable rows: per-row `can_view` runs before the cap, so 413 reflects what the caller may receive. `?bom=1` opts into an Excel BOM.
- Failed table loads render the branded `ErrorState` in-region, not a blank page.

Live updates:

```rust
Table::r#for(cx).live_search(true)
```

Search, sort, filter, and pager controls then refresh the grid in place without a full page load. The plain links and forms stay as the no-JS fallback.

Panel wires the bulk checkbox column automatically (`deletable()` defaults to `true`; override to `false` for read-only resources). The destructive submit stays disabled until at least one row is checked.

---

## 6. Forms

Forms use typed lenses, not string paths:

```rust
Schema::new((
    Section::new("Account").schema((
        TextInput::r#for(User::fields().email()).email().unique(),
        Select::r#for(User::fields().role())
            .options(vec!["admin".into(), "member".into()]),
    )),
    Grid::new(2).schema((
        TextInput::r#for(User::fields().name()),
        FileUpload::r#for(Post::fields().image_path()),
    )),
))
```

What to know:

- Layout blocks: `Section`, `Group`, `Grid`, `Tabs`, `Wizard`. Fields: `TextInput`, `Select`, `FileUpload`, `Repeater`. Every field takes a typed lens (`User::fields().email()`), never a string path.
- `required` defaults to the column nullability. Use `.optional()` to opt out. A bare `Select` over a non-nullable FK rejects `""` inline instead of failing at the driver.
- `unique()` adds an app-level pre-check only. Toasty exposes no unique-violation predicate yet, so the DB constraint stays the final guard and concurrent writes can race.
- Relation select validates the FK against the related resource query before `create_record` runs:

```rust
Select::r#for(Post::fields().author_id())
    .relationship::<AuthorResource>(
        AuthorResource::query,
        |a: &Author| a.id,
        |a: &Author| a.name.clone(),
    )
    .label("Author")
```

- Relation options are bounded to 200 (`MAX_RELATIONSHIP_OPTIONS`) and memoized per `(request, tenant)`. Small tables validate against the bounded set; `can_view` filters before labels, `can_view_any`/tenant denial fails closed (`not available`).
- Large reference tables (10k+ rows) need `.searchable()` on the `Select` (GH #150): over-cap searchable selects degrade to type-to-search instead of a retry error. Typing fetches `GET {parent_list_url}/options?field=&q=` (debounced 200ms, abort in-flight, selection preserved), which reuses the related `Table`'s declared `searchable()` columns (`search_expr`), bounds to 200, and filters `can_view` before labels. No searchable columns → hard-cap path (non-searchable keeps the cap error). Overflowed submits validate via a targeted PK check (`R::query` + `can_view`): legitimate FKs beyond the cap pass, hidden → `invalid`, denied → `not available`, DB failure → retry. Initial render keeps the stored value + search input + “Too many options — type to search” hint; no-JS keeps the plain select (other fields still submit, relation cannot be changed past the cap).

- `FileUpload` stores the sanitized basename as the `String` path (bytes are not persisted in v1; no `value` on `type=file`). Forms with one emit `enctype="multipart/form-data"`. Bodies are capped at 10 MiB (413), multipart without a boundary is a 400, and filenames rejecting `.` / `..` / Windows reserved names surface as inline errors. Empty submits keep the stored path; `clear_<field>=1` opts back into clearing.
- `Repeater` is a single-entry group. An all-empty group is skipped, so its inner required fields do not fail the submit. A `required` repeater yields one label-keyed error; a partially filled group still enforces inner `required`.

Validation errors render inline per field. Absent keys validate as `""` and updates write only present keys; handlers reject unknown form keys with 400 (`role` / `tenant_id` smuggling fails closed; only `csrf_token` and `clear_<field>` are exempt), so extra posted keys never reach record fns.

---

## 7. Policy, auth, tenancy

Policy is `can_*` on the resource, default deny. Check both pages and handlers:

```rust
fn can_view_any(_cx: &Cx) -> bool { true }
fn can_view(_cx: &Cx, _r: &User) -> bool { true }
fn can_create(_cx: &Cx) -> bool { false }
```

List scope belongs in `query()`. Per-row `can_view` trims option lists and exports, but the list page itself checks only `can_view_any` so pagination stays honest. Edit GET and POST both require `can_view` + `can_update`; relation option loads fail closed when the related resource denies `can_view_any`.

Mutations run in a framework-owned transaction: handlers load through `query()` and policy-check on that snapshot, then pass the checked records into the record fns with no silent re-loads. Bulk delete is all-or-nothing.

Auth is on by default and fails closed:

- Register the shipped models and seed one admin:

```rust
toasty::models!(crate::User, argentum_core::auth::AdminUser, argentum_core::auth::AuthSession)
```

```rust
let hash = argentum_core::auth::hash_password("secret").expect("hash password");
// store in AdminUser.password_hash (Argon2id PHC string)
```

- Unauthenticated `GET` pages redirect to `{prefix}/login` with a validated same-origin `next`. Runtime endpoints (`/_topcoat/runtime`) and all non-GET panel requests answer 401; users without panel access answer 403.
- Sessions are server-side `AuthSession` rows with a seven-day fixed lifetime, rotated on login and revoked on logout. Use `auth::revoke_sessions_for_user(cx, id)` to sign a user out everywhere. Logins verify Argon2id (dummy hash for unknown emails) and share one generic failure message. Handlers re-check the resolved user, including the panel root and live-search shard; logout accepts any resolved identity so a de-permitted session can still be cleared.

Custom user table:

```rust
Panel::new("admin").auth(Auth::custom(MyAuth))
```

Implement `verify` plus `find_by_id` for your model. Session storage stays framework-owned. Read the result with `current_user(cx)` or `require_authenticated(cx)`.

Explicit opt-out:

```rust
Panel::new("admin").auth(Auth::disabled())
```

Tenancy comes from the logged-in user. `tenant_id(cx)` reads the request `Tenant`, which the auth layer sets; a server-set `Tenant` request extension overrides deliberately (for middleware/tests). Scope `query()` with it and mark tenant-owned resources with `requires_tenant()` so handlers fail closed (403) without one. Never trust a tenant header from the client.

No built-in rate limiter or lockout: enforce at the edge (proxy/WAF). `Notification` is a one-time `__Host-argentum_notification` flash cookie on the 303 Post/Redirect/Get response, consumed on follow-up so reloads never replay it.

---

## 8. Data access

Get the DB from app context:

```rust
let mut db = argentum_core::db::db(cx);
let rows = User::all().exec(&mut db).await?;
```

`Db` is `Arc`-pooled; cloning per request is cheap and `exec` needs `&mut Db`.

Filter and sort:

```rust
User::filter(User::fields().email().eq("alice@example.com"))
User::filter(User::fields().name().starts_with(q))
    .order_by(User::fields().name().asc())
```

Table search builds its pattern through `like_with_escape` with `%`, `_` and the escape character escaped (`escape_like_pattern`), so it is parameterised and portable. If you hand-write a pattern, escape `%` and `_` first, and never interpolate raw input into SQL.

Preload relations in one trip:

```rust
let posts = Post::all()
    .include(Post::fields().author())
    .exec(&mut db)
    .await?;
// then `post.author.get()` with no extra query
```

Guard computed cells against a missing preload so a query change fails loudly, not with blank data:

```rust
TextColumn::computed("Author", |p: &Post| {
    if p.author.is_unloaded() { "(unloaded)".into() } else { p.author.get().name.clone() }
})
```

Schema setup: `db.push_schema().await` for prototypes, `toasty-cli` migrations for prod.

Render invariants: pages, layouts, and components are side-effect free and deterministic — no `HashMap` iteration, `Utc::now()`, or random IDs in a streamed region (breaks concurrent/streaming re-renders). Query Toasty directly with explicit `include` for relations, `#[index]` for filter columns, and `#[memoize]` for shared loads. `Cx`-scoped values (`Tenant`, auth), not middleware, carry request scope. Every value read on the server via `get()` / `read()` is untrusted client input.

Reactivity: `signal(cx, init)` runs only in a page/layout/component body; loop bodies need `#[key(...)]` and reorderable rows need a stable `id` from the row key. `get()` / `read()` re-runs track and morph in place; `get_untracked()` opts out. Page/layout guards do not run on shard requests — shards authorize themselves (`requires_tenant` + `can_view_any` + `query()` scoping).

---

## 9. Security defaults

- All POSTs verify a double-submit `csrf_token` before any DB work. `confirm=1` is a UX step, not a boundary.
- Passwords use Argon2id. Unknown emails take the same code path, and login failures share one generic message.
- Deletes and bulk deletes re-fetch through `query()` and re-check policy inside the handler transaction.
- Table free-text only reaches `starts_with`. Do not interpolate raw input into SQL.
- Session, CSRF, and notification cookies use hardened `__Host-` + `Secure` settings. Localhost is exempt; non-localhost deploys need HTTPS or browsers drop them and mutations 403.
- Responses carry `Content-Security-Policy: frame-ancestors 'self'`, so an admin page cannot be clickjacked from another origin. `Panel::frame_ancestors(..)` widens it for a deployment that frames the panel, `Panel::without_frame_ancestors()` sends none for a proxy that owns the whole policy, and an app's own `Content-Security-Policy` always wins — the layer only fills the gap.
- Redirects: `Err(redirect(..))` (307) for GETs, `Err(see_other(..))` (303 PRG) after mutations. Mid-stream they degrade to `window.location.replace`; streamed regions own their failure rendering. Wrap `Slot` in `error_boundary` for branded error pages.

---

## 10. Testing and benchmarks

Unit render with `CxTestBuilder`. Cover each resource policy fn. Cover scoping in `query()`.

The showcase has integration tests per area (list, create, edit, delete, bulk, filters, tenancy, uploads, export, auth) under `examples/showcase/tests/`.

```sh
cargo test -p showcase
cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench
./benchmarks/scripts/bench.sh
```

Budget: 50-row list with 2 preloaded relations renders under 40ms p50 on local SQLite. The skeleton ships first, rows stream in after.

---

## 11. Roadmap

Done:

- CRUD for single resources: typed tables, cursor pagination, search and sort, create and edit forms, row and bulk delete, flash notifications, sidebar shell
- Relations: preloaded `BelongsTo` and `HasMany`, relation selects, tenant scoping, server-side option search past the cap
- Table extras: typed filters, page-local grouping with counts, CSV export, live in-place search and sort, empty and error states
- Auth: default login plus sessions, custom user table seam, explicit opt-out, per-resource policy

Next:

- Widgets and infolists: stats overview, charts, global search
- Nicer media handling for uploads
- Documented production migrations

Non-goals for v1:

- Many-to-many helpers in tables
- DynamoDB-backed admin
- SQL aggregates beyond row counts
- WASM admin or SPA mode

---

## 12. Docs

- `CONTEXT.md`: domain vocabulary
- `docs/adr/`: design notes
- `examples/showcase/`: runnable reference
- `benchmarks/README.md`: perf setup
- Upstream: Toasty guide (queries, filters, sorting, preloading, migrations) and Topcoat docs (view and component, router, cookie and session)
- Filament PHP docs as product inspiration, not API source

Notes for contributors: `argentum-ui` primitives mirror `topcoat-ui-registry` verbatim — sync with `cargo xtask sync-topcoat-ui`, never hand-edit; composites (`Page`, `Toast`, `Theme`, `ErrorState`, `CodeBlock`, `BoundInput`) are owned. Topcoat 0.8 / Toasty 0.10 track `main` with pins in `Cargo.lock` — bump deliberately. Where this file and the code disagree, the code and `docs/adr/` win.
