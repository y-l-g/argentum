# Plan 006: Make the `auth`-off build fail closed and keep its tests compiling

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat cbb738a8..HEAD -- crates/argentum-core/src/lib.rs crates/argentum-core/src/panel/ crates/argentum-core/src/notification.rs crates/argentum-core/tests/ .github/workflows/ci.yml AGENTS.md CONTRIBUTING.md .agents/skills/check/SKILL.md docs/adr/0013-panel-auth.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW–MED — breaking for apps that build `argentum-core` with `default-features = false`: they must add one call
- **Depends on**: none (plan 005 adds tests that use `.auth(crate::Auth::disabled())`; whichever lands second re-runs step 4's command)
- **Category**: security + tests
- **Planned at**: commit `cbb738a8`, 2026-09-24

## Why this matters

`argentum-core` has one feature, `auth`, on by default. With it on, turning auth
off is explicit and greppable: `Panel::auth(Auth::disabled())` (ADR-0013 calls it
"the explicit fail-open opt-out"). With the feature **off** — e.g. an app sets
`default-features = false` only to drop the Argon2/session dependencies —
`enforce_auth` becomes a no-op, no gate or login route is installed, and the
panel serves every page and mutation to anyone, with no build error, no log line,
and nothing to grep for. Separately, the test suite cannot even compile in that
mode (`cargo check -p argentum-core --no-default-features --tests` fails with
~85 errors: tests call `.auth(crate::Auth::disabled())`, and `Auth` does not
exist without the feature), and CI only runs `cargo check` (no test code), so the
opt-out path's runtime behavior — CSRF still enforced, tenancy still failing
closed — is never exercised.

The fix: in feature-off builds, provide a minimal `Auth` with only `disabled()`
and a `Panel::auth` that accepts it, and make `Panel::build` return an error
unless the app called `.auth(Auth::disabled())`. That makes the fail-open state
explicit in both builds, and it makes most test code compile unchanged.

## Current state

- `crates/argentum-core/Cargo.toml`:

  ```toml
  [features]
  default = ["auth"]
  auth = ["dep:argon2", "dep:password-hash", "topcoat/session"]
  ```

- `crates/argentum-core/src/lib.rs:20-35`:

  ```rust
  #[cfg(feature = "auth")]
  pub mod auth;
  …
  #[cfg(feature = "auth")]
  pub use auth::{Auth, Authenticator, CurrentUser, PasswordAuth};
  ```

- `crates/argentum-core/src/auth.rs` (feature-on only) defines
  `pub enum Auth { Password(..), Custom(..), Disabled }` with `Auth::disabled()`
  (line ~324) and `is_disabled()` (~340), `Default` = password.
- `crates/argentum-core/src/panel/mod.rs`:
  - struct fields (~line 93-96): `#[cfg(feature = "auth")] login_hint: Option<String>`,
    `#[cfg(feature = "auth")] auth: crate::auth::Auth`; initialized in `Panel::new` (~line 150-153).
  - `#[cfg(feature = "auth")] pub fn auth(mut self, auth: crate::auth::Auth) -> Self` (~line 389).
  - `Panel::build` (~line 413) destructures `self` (with the two cfg'd fields),
    returns `topcoat::Result<Router>`, and reports configuration errors as
    `Err(std::io::Error::other(format!("Panel::build: …")).into())` — e.g.:

    ```rust
    let db = db.ok_or_else(|| {
        topcoat::Error::from(std::io::Error::other(
            "Panel::build requires a Db via app_context",
        ))
    })?;
    ```

  - `enforce_auth` (~line 818-830):

    ```rust
    #[cfg(feature = "auth")]
    pub(crate) fn enforce_auth(cx: &Cx) -> Result<(), topcoat::Error> {
        if crate::auth::enforced(cx) {
            crate::auth::require_authenticated(cx)?;
        }
        Ok(())
    }

    /// Auth compiled out: the gate does not exist either, so nothing to enforce.
    #[cfg(not(feature = "auth"))]
    pub(crate) fn enforce_auth(_cx: &Cx) -> Result<(), topcoat::Error> {
        Ok(())
    }
    ```

- Other `cfg(feature = "auth")` sites: `panel/shell.rs:268,294`,
  `notification.rs:288`, `panel/mod.rs:37,111,478,498,570`.
- Test code that references auth-only items (compile errors in feature-off mode):
  `panel/search.rs:226,271` (`crate::auth::RUNTIME_PREFIX`), `panel/mod.rs:~1047`
  (`crate::auth::CurrentUser`), `crates/argentum-core/tests/auth_override.rs`
  (whole module), plus any `use argentum_core::{Auth, auth, …}` in `crates/argentum-core/tests/*.rs`.
  `crates/argentum-core/tests/it.rs` lists the integration modules (`mod after_commit; mod auth_override; …`).
- CI (`.github/workflows/ci.yml:31-34`), in the `test` job:

  ```yaml
      # The auth feature ships on by default and is opt-out; keep the opt-out
      # path compiling (and warning-free) so `default-features = false` stays
      # a real escape hatch (GH #129).
      - run: cargo check -p argentum-core --no-default-features --locked
  ```

- The same command is gate 3 in `AGENTS.md` (Commands block),
  `CONTRIBUTING.md:42` (+ the paragraph at ~line 51-53), and
  `.agents/skills/check/SKILL.md:15`.
- `docs/adr/0013-panel-auth.md` Consequences (~line 45-50) says the Argon2 and
  session dependencies are "opt-out via `default-features = false`" and does not
  mention that auth enforcement disappears with them.

Conventions: build-time misconfiguration is an `Err` from `Panel::build`, never a
panic (GH #174). Rustdoc explains *why*; docs describe current behavior only.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Feature-off tests | `cargo test -p argentum-core --no-default-features --locked` | exit 0 |
| Feature-on tests | `cargo test --workspace --locked` | exit 0 |
| Lint (both) | `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo clippy -p argentum-core --no-default-features --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo +nightly fmt --all -- --check` | exit 0 |
| MSRV | `cargo +1.98 check --workspace --locked` | exit 0 |
| Unused deps | `cargo +nightly udeps --workspace --all-targets --all-features --locked` | exit 0 (if installed) |

## Scope

**In scope**:
- `crates/argentum-core/src/lib.rs`, `crates/argentum-core/src/auth_off.rs` (create),
  `crates/argentum-core/src/panel/mod.rs`, and `#[cfg]` attributes on test code in
  `crates/argentum-core/src/**` and `crates/argentum-core/tests/**`
- `.github/workflows/ci.yml` (one step), `AGENTS.md`, `CONTRIBUTING.md`,
  `.agents/skills/check/SKILL.md` (gate 3 text), `docs/adr/0013-panel-auth.md`

**Out of scope**:
- `crates/argentum-core/src/auth.rs` behavior (feature-on auth is unchanged).
- The showcase and benchmarks (they build with default features).
- Making the feature-off `Auth` support anything but `disabled()`.

## Git workflow

- Branch: `advisor/006-auth-off-fails-closed`.
- Squash-merged as `fix(auth)!: refuse to build an ungated panel unless it opts out explicitly (#<issue>)`
  with a `BREAKING CHANGE:` footer: "with `default-features = false`, call
  `Panel::auth(Auth::disabled())` before `build()`."
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: A feature-off `Auth`

Create `crates/argentum-core/src/auth_off.rs`:

```rust
//! The `auth` feature compiled out (ADR-0013): no sessions, no login, no gate.
//! What remains is the one value that says so. `Panel::build` refuses a panel
//! that has not been handed it, so an ungated panel is always a line of app
//! code, never a side effect of trimming dependencies.

/// Authentication configuration when the `auth` feature is off: only the
/// explicit opt-out exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Auth(());

impl Auth {
    /// Serve the panel without authentication. Every page and mutation answers
    /// anyone who can reach it.
    #[must_use]
    pub fn disabled() -> Self {
        Self(())
    }

    /// Always `true`: the feature-off build has no other configuration.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        true
    }
}
```

In `lib.rs` add, next to the feature-on lines:

```rust
#[cfg(not(feature = "auth"))]
mod auth_off;
#[cfg(not(feature = "auth"))]
pub use auth_off::Auth;
```

**Verify**: `cargo check -p argentum-core --no-default-features --locked` → exit 0.

### Step 2: `Panel::auth` and the build-time refusal

In `panel/mod.rs`:
- add a field `#[cfg(not(feature = "auth"))] auth_disabled: bool` (init `false`
  in `Panel::new`, destructure it in `build`);
- add

  ```rust
  /// Acknowledge that this build has no authentication (ADR-0013): with the
  /// `auth` feature off, [`build`](Self::build) refuses a panel that has not
  /// been handed [`Auth::disabled`](crate::Auth::disabled).
  #[cfg(not(feature = "auth"))]
  pub fn auth(mut self, _auth: crate::Auth) -> Self {
      self.auth_disabled = true;
      self
  }
  ```

- in `build`, right after the `registration_errors` check, add

  ```rust
  #[cfg(not(feature = "auth"))]
  if !self.auth_disabled {
      return Err(std::io::Error::other(
          "Panel::build: argentum-core is built without the `auth` feature, so nothing \
           authenticates requests; call `.auth(Auth::disabled())` to serve the panel \
           ungated, or enable the feature",
      )
      .into());
  }
  ```

  (Place it before `self` is destructured, or read the destructured binding.)
- update the rustdoc `# Errors` list of `build` to mention it.

**Verify**: `cargo check -p argentum-core --no-default-features --locked` → exit 0,
and `cargo check -p argentum-core --locked` → exit 0.

### Step 3: Make the test code compile in both modes

Run `cargo test -p argentum-core --no-default-features --locked --no-run` and fix
each remaining error by gating, never by deleting a test:
- a test (or helper) that exercises login, sessions, `CurrentUser`,
  `RUNTIME_PREFIX` or other `crate::auth` items → `#[cfg(feature = "auth")]` on
  that test fn or helper;
- `crates/argentum-core/tests/it.rs`: `#[cfg(feature = "auth")] mod auth_override;`
  (and any other module that is wholly about auth);
- an import list mixing auth and non-auth items → split the auth items into a
  `#[cfg(feature = "auth")] use …;` line.

Tests that only call `.auth(crate::Auth::disabled())` / `.auth(Auth::disabled())`
must compile **unchanged** thanks to steps 1–2 — do not touch them.

A test that asserts auth-on behavior (e.g. an anonymous request is redirected to
login) fails at runtime, not compile time, in the feature-off build: gate it with
`#[cfg(feature = "auth")]` when step 4 shows it failing for that reason.

**Verify**: `cargo test -p argentum-core --no-default-features --locked --no-run` → exit 0.

### Step 4: Feature-off tests pass, and three new ones

Add to `panel/mod.rs` `mod tests`, all `#[cfg(not(feature = "auth"))]`, modeled on
the existing build tests there (search for `.build()` + `expect_err` / `is_err`):
1. `build_refuses_an_unacknowledged_ungated_panel` — a panel with a `Db` and one
   resource, no `.auth(..)` → `build()` is `Err` and its message contains `auth`.
2. `build_accepts_the_explicit_opt_out` — same with `.auth(crate::Auth::disabled())` → `Ok`.
3. `csrf_is_enforced_without_the_auth_feature` — with the opt-out, a url-encoded
   POST to the resource's create route with no `csrf_token` → 403 (copy the
   request shape from an existing create/CSRF test in `panel/forms.rs` or
   `panel/actions.rs` tests).

**Verify**: `cargo test -p argentum-core --no-default-features --locked` → exit 0.
**Verify**: `cargo test --workspace --locked` → exit 0 (feature-on unchanged).

### Step 5: CI and gate docs

- `.github/workflows/ci.yml`: change the step to
  `cargo test -p argentum-core --no-default-features --locked` and reword its
  comment: the opt-out path must compile *and* pass its tests, and `Panel::build`
  refuses it without `Auth::disabled()`.
- `AGENTS.md` (Commands block), `CONTRIBUTING.md` (gate 3 and the paragraph
  explaining it) and `.agents/skills/check/SKILL.md`: replace
  `cargo check -p argentum-core --no-default-features --locked` with the `cargo test`
  form. Keep the list at ten gates.
- `docs/adr/0013-panel-auth.md` Consequences: state in present tense that with
  `default-features = false` the panel has no gate, and `Panel::build` refuses to
  build it unless handed `Auth::disabled()` — the same explicit opt-out the
  default build uses. Add today's date to the header's `Amended:` list.

**Verify**: `grep -rn "check -p argentum-core --no-default-features" AGENTS.md CONTRIBUTING.md .agents .github` → no matches.

## Test plan

- New (feature-off only): build refusal, explicit opt-out, CSRF still enforced.
- Whole existing suite runs in both feature modes; auth-specific tests gated, not removed.

## Done criteria

- [ ] `cargo test -p argentum-core --no-default-features --locked` exits 0
- [ ] `cargo test --workspace --locked` exits 0
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo clippy -p argentum-core --no-default-features --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo +nightly fmt --all -- --check` exits 0
- [ ] `cargo +1.98 check --workspace --locked` exits 0
- [ ] `git diff cbb738a8 -- crates/argentum-core | grep -E '^-\s*(pub(\(crate\))? )?(async )?fn '` prints nothing (no function was removed; tests were gated, not deleted)
- [ ] `plans/README.md` status row updated

## STOP conditions

- More than ~15 tests need a `#[cfg(feature = "auth")]` gate in step 3/4 for
  runtime (not compile) reasons — the feature-off build may behave differently
  than this plan assumes; report the list.
- `cargo +nightly udeps` flags a dependency as unused in either mode.
- A downstream crate in this workspace (showcase, benchmarks, xtask) turns out to
  build `argentum-core` without default features.
- The maintainer's intent for the feature-off build turns out to be "silently
  ungated by design" (e.g. a doc or issue says so): stop before step 2.

## Maintenance notes

- If more `Auth` constructors are ever wanted without the feature, that is a
  design change to ADR-0013 — the feature-off `Auth` is deliberately a single value.
- Reviewers: check that no test was deleted, only gated, and that the feature-off
  `Panel::auth` signature matches the feature-on one closely enough that
  `.auth(Auth::disabled())` reads identically in app code.
- Plan 005's new tests use `.auth(crate::Auth::disabled())` and therefore compile
  in both modes; any test it adds that asserts login behavior needs the gate.
