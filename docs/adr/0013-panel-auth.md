# Authentication ships with the panel — server-side sessions, one override seam

Date: 2026-09-10 — Status: accepted — Supersedes: none

## Context

Argentum shipped authorization (`can_*`, default-deny, checked in page and POST handlers) and a `Cx`-scoped tenancy seam (`Tenant`, `Resource::query`) but no authentication. Every resource route and runtime endpoint is public unless an app overrides `can_*` — the showcase overrides them to `true`, so `/admin/users` is open to anyone. There is no login, session, current user, or password hashing. Tenant-scoped resources are unreachable in a browser: the only request-time tenant source is the `x-tenant-id` header, documented as test/showcase-harness-only, so `/admin/authors` and `/admin/posts` fail closed with 403. Every app adopting Argentum would hand-roll login, credential storage, hashing, sessions, cookie handling, and gate logic, contradicting the "CRUD core of Filament, out of the box" promise.

Topcoat provides session mechanics only — a random token, its SHA-256 hash, cookie transport, and an app-owned storage seam — plus a native `Layer` hook that derives a child `Cx`, and lists Authentication as unimplemented upstream. ADR-0010 and README §2 already reject Tower middleware for business logic in favor of `Cx`-scoped values plus `fn require_*` helpers, and the session/cookie layers Topcoat ships use exactly that native layer pattern.

## Decision

Authentication is a first-class, default-on `Panel` concern in `argentum-core`, behind a feature that ships enabled:

- **One override seam:** an object-safe `Authenticator` trait (boxed futures) is stored per `Panel` via `Panel::auth(...)`, erased to a boxed trait object. The default `PasswordAuth` authenticates against a shipped `AdminUser` model; an app with its own user table implements the same trait. `Auth::disabled()` is the explicit fail-open opt-out; the default is gated. `Panel` gains no type parameter.
- **One erased identity:** resolution places `CurrentUser { id, login, display_name, tenant_id, can_access_panel }` in request `Cx`; pages and shards read it through `current_user`/`require_authenticated` only.
- **Server-side sessions:** Topcoat's token transport plus an `AuthSession` Toasty table keyed by token hash. Seven-day fixed lifetime, rotated on login, deleted on logout, revocable per user so deactivation and the future password-reset spec have a correct revoke-all path.
- **Fail closed:** the panel is gated by default. Unauthenticated page requests redirect to `{prefix}/login?next=` (same-origin relative only); runtime endpoints answer 401; valid credentials without panel access get the same 403 as a bad password. An auth layer covers the panel prefix and the runtime prefix; panel handlers and shards additionally call `require_authenticated` (defense in depth, mirroring the shards-authorize-themselves invariant).
- **Argon2id** with PHC-string storage is the password default; login runs a verification even for unknown users, so errors and timing do not enumerate accounts.
- **Tenancy stays orthogonal:** the resolved user optionally exposes `tenant_id`; the auth layer injects `Tenant` into the same child `Cx` only when present, and core auth never requires a tenant. The `x-tenant-id` header fallback leaves the production path; tests inject `Tenant` through request extensions.
- **Scope:** password reset is not in v1 (separate spec), and brute-force limiting stays a deployment concern — an in-process limiter would be false safety across instances, and account lockout is a DoS against the real admin.

Considered: (A) stateless signed-cookie sessions (rejected: no revocation), (B) account lockout after N failures (rejected: DoS against the legitimate admin), (C) requiring apps to hand-roll auth over generic Topcoat sessions (rejected: the out-of-the-box promise), (D) making `Panel` generic over the user type (rejected: poisons every framework type; an erased value suffices), (E) a built-in in-process rate limiter (deferred: wrong layer in multi-instance deployments).

## Consequences

- A fresh app registers the shipped models, seeds an `AdminUser`, and gets a working login and a gated panel; an existing app implements one trait and swaps it in. The showcase proves the default path end-to-end, and a core integration test proves the override path.
- The showcase's `/admin/authors` and `/admin/posts` become reachable by logging in — the tenant comes from the user — instead of by an `x-tenant-id` header; removing that fallback closes a documented spoofing footgun.
- `argentum-core` grows its first production Toasty models and its first feature flag. The Argon2 and session dependencies are opt-out via `default-features = false`.
- Browser auth flows stay on the `Ok` response path until upstream Topcoat #126 (Set-Cookie on error responses) is fixed; the test suite pins that behavior.
- Password reset, registration, 2FA, multi-panel guards, roles/RBAC, and the `Policy` wire-or-remove decision (#109) remain open, each with a seam that does not need reopening: per-user session revocation, per-`Panel` `Auth` values, and `can_access_panel`.
