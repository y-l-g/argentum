# Plan 005: Pin every route's auth, CSRF, tenant and policy gates with tests

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat cbb738a8..HEAD -- examples/showcase/tests/ crates/argentum-core/src/panel/search.rs crates/argentum-core/src/panel/mod.rs examples/showcase/src/app.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW — tests only; no production code changes
- **Depends on**: none. Should land **before** any refactor of the panel handlers' loaders (e.g. narrowing `Resource::query` includes per loader).
- **Category**: tests
- **Planned at**: commit `cbb738a8`, 2026-09-24

## Why this matters

The handlers are correct today — an audit traced every route to its gates — but
several gates have no test, so a refactor that drops one passes the whole suite:

- CSRF rejection is tested for url-encoded create, edit, delete and the showcase
  media upload, but **not** for bulk delete, login, logout, or the multipart
  create/edit path.
- No test sends a **valid-CSRF** edit or delete POST from tenant B for a record
  tenant A owns; the test named `delete_404_for_missing_or_wrong_tenant` only
  posts a random UUID. If the in-transaction reload ever bypassed the
  tenant-scoped query, cross-tenant writes would go green.
- The live-search shard (`POST /_topcoat/runtime/shards/…`) re-checks auth,
  tenant and `can_view_any` itself (page guards do not run on shard requests),
  but its only tests use an ungated, un-tenanted, allow-all resource.
- The export route's `can_view_any` denial is untested.

This plan adds one showcase test module for the HTTP gates and core unit tests
for the shard gates. No production code changes.

## Current state

Routes the panel registers per resource (`crates/argentum-core/src/panel/mod.rs:255-330`),
with `{prefix}` = `/admin`:

| Method | Path | Handler gate order |
|---|---|---|
| GET | `/admin/{slug}` | auth → tenant → `can_view_any` |
| GET/POST | `/admin/{slug}/create` | auth → tenant → parse → CSRF → `can_create` … |
| GET | `/admin/{slug}/{id}` | detail |
| GET/POST | `/admin/{slug}/{id}/edit` | POST: auth → tenant → parse → CSRF → scoped load (404) → `can_view`/`can_update` (403) |
| POST | `/admin/{slug}/{id}/delete` | CSRF + `confirm=1` before any DB work |
| POST | `/admin/{slug}/bulk-delete` | CSRF + `confirm=1`, body `ids=a,b&confirm=1&csrf_token=…` |
| GET | `/admin/{slug}/export` | auth → tenant → `can_view_any` |
| GET | `/admin/{slug}/options` | relationship option search |
| POST | `/admin/login`, `/admin/logout` | CSRF (`crates/argentum-core/src/auth.rs:675-680, 753-765`); login fields `email`, `password`, `csrf_token` |

Showcase test infrastructure (`examples/showcase/tests/`):
- One test binary: `examples/showcase/tests/it.rs` lists every module (`mod admin; mod auth_check; …`). A new file must be added there.
- `common/mod.rs` helpers: `full_db()`, `tenanted_db() -> (Db, t1, t2)` (one author, one
  post, one comment per tenant; the demo admin's own tenant is `DEMO_TENANT`),
  `router(db)` (`showcase::app::router`) / `router_for_tests(db)` (no assets, no uploader),
  `demo_client(&router, &db)` (minted session), `TestClient::new(&router)` (anonymous),
  client builders `.csrf(token)` (sets the CSRF cookie), `.tenant(uuid)`
  (request-extension tenant override), `.get(uri)`, `.post_form(uri, body)`,
  `.post_multipart(uri, boundary, body)`; `post_count(&db)`, `comment_count(&db)`,
  `body_string(resp)`, `SESSION_COOKIE`, `set_cookie_header(&resp, name)`,
  `session_cookie_value(&resp)`.
- `showcase::models::BLOCKED_TENANT` — `PostResource::can_view_any` returns `false`
  for it (`examples/showcase/src/app.rs:593`).
- Exemplars:
  - CSRF 403 pattern: `edit_check.rs:131` (`edit_rejects_forged_post_before_probing_the_record`) — posts with a cookie token that differs from the field (`client.csrf(&cookie_mismatch)` + `csrf_token={csrf}`) and with no field at all.
  - Cross-tenant pattern: `tenancy_check.rs:289` (`bulk_delete_wrong_tenant_404s_and_deletes_nothing`) — `tenanted_db()`, `client.tenant(t2).csrf(&csrf).post_form(..)`, then re-query with `Post::filter(Post::fields().tenant_id().eq(t1))`.
  - Anonymous pattern: `auth_check.rs:302-350` — anonymous GET → 307 to `/admin/login?next=…`; anonymous mutation POST → **401**.
  - Multipart framing: `upload_check.rs:40-50`.

Core shard tests (`crates/argentum-core/src/panel/search.rs`, `mod tests` from ~line 220):
the test at ~line 330-440 builds a `LiveResource` over a local `Dummy` model with
`.live_search(true)`, a panel with `.auth(crate::Auth::disabled())`, and posts:

```rust
let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
let shard_args = |cursor: &str, group_by: &str| {
    format!(
        r#"["/admin/dummies",{}, {}, {}, {}, {}, {}, {}]"#,
        sig(1, ""), sig(2, ""), sig(3, ""), sig(4, ""), sig(5, cursor), sig(6, group_by), sig(7, ""),
    )
};
let shard = topcoat::runtime::Shard::id(&table_search);
let response = router.handle(
    http::Request::builder()
        .method(http::Method::POST)
        .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
        .body(Body::from(format!(r#"{{"args":{},"signals":{{}}}}"#, shard_args("", ""))))
        .unwrap(),
).await;
```

Tenancy: a resource opts in with `fn requires_tenant() -> bool { true }`; the
framework derives the filter from a model column named `tenant_id` of type
`uuid::Uuid` (`crates/argentum-core/src/tenancy.rs`, `TENANT_FIELD`). A request
carries a tenant as an `http` extension: `request.extensions_mut().insert(crate::tenancy::Tenant(id))`.

Rules (`docs/dev/TESTING.md`): assert status codes, redirects and DB state, not
copy; every test names the regression that would make it fail.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| New showcase module | `cargo test -p showcase --test it gate_matrix_check::` | all pass |
| Shard tests | `cargo test -p argentum-core --lib search` | all pass |
| Whole workspace | `cargo test --workspace --locked` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo +nightly fmt --all -- --check` | exit 0 |

## Scope

**In scope**:
- `examples/showcase/tests/gate_matrix_check.rs` (create)
- `examples/showcase/tests/it.rs` (register the module)
- `examples/showcase/tests/delete_check.rs` (rename one test, step 5)
- `crates/argentum-core/src/panel/search.rs` (`mod tests` only)

**Out of scope**: any non-test code. If a test fails because a gate is actually
missing, that is a STOP condition, not something to fix here.

## Git workflow

- Branch: `advisor/005-route-gate-test-matrix`.
- Squash-merged as `test(panel): pin every route's auth, CSRF, tenant and policy gates (#<issue>)`.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Create the module

Create `examples/showcase/tests/gate_matrix_check.rs` with a `//!` header (one
paragraph: what the module pins and why — gates restated per route so a refactor
that drops one fails here) and add `mod gate_matrix_check;` to `it.rs` in
alphabetical order. Import from `crate::common::*` what you use, and
`showcase::app::router_for_tests as router` (no uploader: multipart file parts
are drained, nothing touches the filesystem).

**Verify**: `cargo test -p showcase --test it gate_matrix_check::` → compiles, 0 tests.

### Step 2: CSRF rejection on every state-changing route

Add `forged_posts_answer_403_and_change_nothing`. Setup: `full_db()`, `router`,
`demo_client`. Look up one existing `Post` (any) for `{id}`. For each case below,
send it twice — (a) cookie token ≠ field token, (b) no `csrf_token` field — and
assert **403** plus the listed "unchanged" check:

1. url-encoded `POST /admin/posts/{id}/delete` body `confirm=1&csrf_token=…` → `post_count` unchanged.
2. url-encoded `POST /admin/posts/bulk-delete` body `ids={id}&confirm=1&csrf_token=…` → `post_count` unchanged.
3. multipart `POST /admin/posts/create` with text parts `title`, `author_id`, `csrf_token` and a file part `image_path` (`filename="x.png"`) → `post_count` unchanged.
4. multipart `POST /admin/posts/{id}/edit` with `title=Forged` + file part → the post's `title` unchanged.

Add `forged_login_answers_403_and_sets_no_session`: anonymous
`TestClient::new(&router).csrf(&cookie_token)` posting
`email=<DEMO_ADMIN_EMAIL>&password=<DEMO_ADMIN_PASSWORD>&csrf_token=<other>` to
`/admin/login` (build the body with `form_body(&[..])`) → 403 and
`session_cookie_value(&resp)` is `None`. (The constants are in `showcase::models`;
reference them by name, never paste their values.)

Add `forged_logout_answers_403_and_keeps_the_session`: `demo_client` posting
`csrf_token=<other>` with a mismatched cookie to `/admin/logout` → 403; then the
same client's `GET /admin/posts` is still 200 (the session survived).

**Verify**: `cargo test -p showcase --test it gate_matrix_check::` → all pass.

### Step 3: Cross-tenant writes with a valid token

Add `cross_tenant_edit_and_delete_404_and_touch_nothing`. Setup: `tenanted_db()`,
`router`, `demo_client`, a valid CSRF pair (`let csrf = Uuid::new_v4().to_string();`
then `.csrf(&csrf)` and `csrf_token={csrf}` in the body). Find `t1`'s post. As
`client.tenant(t2)`:
- `POST /admin/posts/{t1_post}/edit` body `title=Hijacked&author_id={t1_author}&csrf_token={csrf}` → **404**; re-query: title unchanged.
- `POST /admin/posts/{t1_post}/delete` body `confirm=1&csrf_token={csrf}` → **404**; the post still exists.
- Repeat both for `t1`'s comment at `/admin/comments/{t1_comment}/edit` and `/delete`
  (comments inherit tenancy through their post) → 404, comment unchanged/present.

Then, as `client.tenant(t1)` (the owner), the same delete of the t1 **comment** →
redirect (3xx) — proves the 404 above is the tenant scope, not a broken route.

**Verify**: `cargo test -p showcase --test it gate_matrix_check::` → all pass.

### Step 4: Read routes under policy denial and anonymity

Add `blocked_tenant_is_refused_on_every_read_route`: `full_db()`, `demo_client`,
`client.tenant(BLOCKED_TENANT)`; GET `/admin/posts`, `/admin/posts/export`,
`/admin/posts/{id}` (any post id) → each **403**. (If the detail answers 404
because the blocked tenant owns no rows, use a post created under
`BLOCKED_TENANT` — insert one with `toasty::create!` copying the `Post` literal
from `common::tenanted_db`.)

Add `anonymous_requests_are_gated_on_every_route`: `TestClient::new(&router)`:
- GET `/admin/posts`, `/admin/posts/create`, `/admin/posts/{id}`,
  `/admin/posts/{id}/edit`, `/admin/posts/export`, `/admin/posts/options?q=a`
  → **307** with `Location` starting `/admin/login`.
- POST `/admin/posts/create`, `/admin/posts/{id}/edit`, `/admin/posts/{id}/delete`,
  `/admin/posts/bulk-delete` (any body) → **401**; `post_count` unchanged.

**Verify**: `cargo test -p showcase --test it gate_matrix_check::` → all pass.

### Step 5: Name the delete test for what it checks

In `examples/showcase/tests/delete_check.rs` (~line 209), rename
`delete_404_for_missing_or_wrong_tenant` to `delete_404_for_an_unknown_id` and
point its comment at `gate_matrix_check::cross_tenant_edit_and_delete_404_and_touch_nothing`.

**Verify**: `cargo test -p showcase --test it delete_check::` → all pass.

### Step 6: Shard gates in core

In `crates/argentum-core/src/panel/search.rs` `mod tests`, add a local model and
two resources (copy `LiveResource`'s shape):

```rust
#[derive(Debug, Clone, toasty::Model)]
struct TenantDummy {
    #[key]
    #[auto]
    id: uuid::Uuid,
    #[index]
    tenant_id: uuid::Uuid,
    name: String,
}
```

- `TenantLive` — `slug()` `"tenant-dummies"`, `requires_tenant() -> true`,
  `can_view_any -> true`, `can_view -> true`, table with `.id`, `.pk`, one
  `TextColumn` on `name`, `.paginate(10)`, `.live_search(true)`.
- `DeniedLive` — same model, `slug()` `"denied-dummies"`, `can_view_any -> false`.

Seed `Alpha` under tenant `A` and `Bravo` under tenant `B`. Build a panel with
both resources and `.auth(crate::Auth::disabled())`. Write a local helper that
posts the shard request above with the first arg set to the resource's list path
and an optional `Tenant` extension. Assert:
- `TenantLive` with no tenant → **403**.
- `TenantLive` with tenant `A` → 200, body contains `Alpha`, not `Bravo`.
- `DeniedLive` with tenant `A` → **403**.

**Verify**: `cargo test -p argentum-core --lib search` → all pass including 3 new assertions groups.

## Test plan

This plan *is* tests. Each new test names, in its doc comment, the regression
it catches (e.g. "bulk delete verifying CSRF after the id fetch").

## Done criteria

- [ ] `cargo test --workspace --locked` exits 0
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo +nightly fmt --all -- --check` exits 0
- [ ] `grep -c "#\[tokio::test\]" examples/showcase/tests/gate_matrix_check.rs` ≥ 6
- [ ] `git diff --stat` shows no file outside `examples/showcase/tests/` and `crates/argentum-core/src/panel/search.rs`
- [ ] `plans/README.md` status row updated

## STOP conditions

- Any new assertion fails against the unmodified production code in a way that
  means a gate is **missing** (e.g. a forged POST answers 3xx/200, a cross-tenant
  edit succeeds, a blocked tenant reads rows). That is a security bug: stop and
  report the route, request and response — do not fix production code in this plan.
- A status differs from the plan's expectation for a benign reason (e.g. the
  shard answers 401 instead of 403 for a missing tenant, or anonymous options
  answers 401): report the observed code rather than loosening the assertion to
  "any 4xx".
- The shard request shape (positional args) no longer matches `table_search`'s
  signature.

## Maintenance notes

- When a new route is added to `Panel::resource` or `auth::install`, add its row
  to this module; reviewers should ask for it.
- A follow-up could enumerate the router's registered routes and fail when a
  route has no row here; Topcoat exposes no route listing today.
