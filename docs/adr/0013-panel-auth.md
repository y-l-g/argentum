# Authentication ships with the panel — server-side sessions, one override seam

Date: 2026-09-10 — Status: accepted — Amended: 2026-09-14

## Decision

Authentication is a first-class, default-on `Panel` concern in `argentum-core`, behind a feature that
ships enabled:

- **One override seam.** An object-safe `Authenticator` trait (boxed futures) is stored per `Panel`
  via `Panel::auth(...)`, erased to a boxed trait object. The default `PasswordAuth` authenticates
  against a shipped `AdminUser` model; an app with its own user table implements the same trait.
  `Auth::disabled()` is the explicit fail-open opt-out; `Panel` gains no type parameter.
- **One erased identity.** Resolution places
  `CurrentUser { id, login, display_name, tenant_id, can_access_panel }` in the request `Cx`; pages and
  shards read it through `current_user`/`require_authenticated` only.
- **Server-side sessions.** Topcoat's token transport plus an `AuthSession` Toasty table keyed by the
  token hash: seven-day fixed lifetime, rotated on login, deleted on logout, revocable per user so
  deactivation and a future password-reset spec have a correct revoke-all path.
- **Fail closed.** The panel is gated by default. Unauthenticated page requests redirect to
  `{prefix}/login?next=` (same-origin relative only); runtime endpoints answer 401; valid credentials
  without panel access get the same 403 as a bad password. An auth layer covers the panel prefix and
  the runtime prefix, and panel handlers and shards additionally call `require_authenticated`
  (defense in depth, mirroring the shards-authorize-themselves invariant).
- **Argon2id** with PHC-string storage is the password default; login verifies even for unknown users,
  so errors and timing do not enumerate accounts.
- **Tenancy stays orthogonal.** The resolved user optionally exposes `tenant_id`; the auth layer
  injects `Tenant` into the same child `Cx` only when present, and core auth never requires a tenant.
- **Scope.** Password reset is not in v1 (separate spec); brute-force limiting stays a deployment
  concern, since an in-process limiter is false safety across instances and account lockout is a DoS
  against the real admin.

Logout is the one route an authenticated-but-no-longer-permitted user may still reach (GH #146): the
gate answers `{prefix}/logout` for any resolved user, and `logout_post` demands a resolved identity
rather than panel permission — clearing the session must not require the access that was just
revoked. The bypass is scoped to the POST method at the exact logout path, and no other handler may
live there. `GET {prefix}/login` is gate-bypassed and re-issues a CSRF token, so a user whose pair
went stale still recovers; with the shipped `PasswordAuth`, deactivation resolves no user at all
(`find_by_id` filters on `active`), so the stranding case exists only for custom `Authenticator`s
whose `find_by_id` keeps resolving a de-permitted user.

## Consequences

A fresh app registers the shipped models, seeds an `AdminUser`, and gets a working login and a gated
panel; an existing app implements one trait and swaps it in. The showcase proves the default path
end-to-end, and a core integration test proves the override path. Tenant-scoped pages become reachable
by logging in — the tenant comes from the user. `argentum-core` grows its first production Toasty
models and its first feature flag, with the Argon2 and session dependencies opt-out via
`default-features = false`. Password reset, registration, 2FA, multi-panel guards, and roles/RBAC
remain open, each with a seam that does not need reopening: per-user session revocation, per-`Panel`
`Auth` values, and `can_access_panel`.

Rejected: stateless signed-cookie sessions (no revocation), making `Panel` generic over the user type
(it would poison every framework type, and an erased value suffices), and requiring apps to hand-roll
auth over generic Topcoat sessions (it breaks the out-of-the-box promise). Account lockout and an
in-process rate limiter are covered by the scope bullet above.
