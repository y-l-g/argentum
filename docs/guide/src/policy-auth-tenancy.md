# Policy, auth, tenancy

Who may see and change what: the `can_*` predicates, the auth gate and its seams, and where the
request's tenant comes from.

Policy is `can_*` on the resource, default deny. Check both pages and handlers:

```rust
fn can_view_any(_cx: &Cx) -> bool { true }
fn can_view(_cx: &Cx, _r: &User) -> bool { true }
fn can_create(_cx: &Cx) -> bool { false }
```

List scope belongs in `query()`. Per-row `can_view` trims option lists and exports, but the list page
itself checks only `can_view_any` so pagination stays honest. Edit GET and POST both require
`can_view` + `can_update`; relation option loads fail closed when the related resource denies
`can_view_any`.

Mutations run in a framework-owned transaction: handlers load through `query()` and policy-check on
that snapshot, then pass the checked records into the record fns with no silent re-loads. Bulk delete
is all-or-nothing. Anything that must happen *after* the commit — a webhook, an email, an audit row —
goes in `after_commit`, which runs once the transaction is gone; see [Resources](./resources.md).

Auth is on by default and fails closed:

- Register the shipped models and seed one admin:

```rust
toasty::models!(crate::User, argentum_core::auth::AdminUser, argentum_core::auth::AuthSession)
```

```rust
let hash = argentum_core::auth::hash_password("secret").expect("hash password");
// store in AdminUser.password_hash (Argon2id PHC string)
```

- Unauthenticated `GET` pages redirect to `{prefix}/login` with a validated same-origin `next`.
  Runtime endpoints (`/_topcoat/runtime`) and all non-GET panel requests answer 401; users without
  panel access answer 403. The gate installs exactly two layers — the panel prefix and
  `/_topcoat/runtime` — so a route mounted outside them is ungated by construction;
  `Panel::serve_dir` is the shipped case, and served directories are public by decision (ADR-0017).
- Sessions are server-side `AuthSession` rows with a seven-day fixed lifetime, rotated on login and
  revoked on logout. Use `auth::revoke_sessions_for_user(cx, id)` to sign a user out everywhere.
  Logins verify Argon2id (dummy hash for unknown emails) and share one generic failure message.
  Handlers re-check the resolved user, including the panel root and live-search shard; logout accepts
  any resolved identity so a de-permitted session can still be cleared.

Custom user table:

```rust
Panel::new("admin").auth(Auth::custom(MyAuth))
```

Implement `verify` plus `find_by_id` for your model. Session storage stays framework-owned. Read the
result with `current_user(cx)` or `require_authenticated(cx)`.

Explicit opt-out:

```rust
Panel::new("admin").auth(Auth::disabled())
```

Tenancy comes from the logged-in user. `tenant_id(cx)` reads the request `Tenant`, which the auth
layer sets; a server-set `Tenant` request extension overrides deliberately (for middleware/tests).
Mark tenant-owned resources with `requires_tenant()`: handlers fail closed (403) without a tenant,
and the framework derives the `tenant_id` filter from the model and applies it at every loader
(GH #223), so no `query()` override has to restate it. Never trust a tenant header from the client.
See [Resources](./resources.md) for the three shapes a resource can declare.

No built-in rate limiter or lockout: enforce at the edge (proxy/WAF). `Notification` is a one-time
`__Host-argentum_notification` flash cookie on the 303 Post/Redirect/Get response, consumed on
follow-up so reloads never replay it.
