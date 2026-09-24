# Plan 002: Serve uploaded files as inert content from `Panel::serve_dir`

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat cbb738a8..HEAD -- crates/argentum-core/src/panel/headers.rs crates/argentum-core/src/panel/mod.rs crates/argentum-core/tests/uploads.rs docs/adr/0017-media-uploads.md docs/guide/src/security.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED — non-image files served from a directory start downloading instead of opening inline
- **Depends on**: none (complements plan 001)
- **Category**: security
- **Planned at**: commit `cbb738a8`, 2026-09-24

## Why this matters

`Panel::serve_dir` mounts an app directory (typically the `Uploader`'s output)
on the **panel's own router**, so its files share the admin's origin. Topcoat's
`DirectoryRoute` derives `Content-Type` from the file extension
(`.html` → `text/html`, `.svg` → `image/svg+xml`) and sends no `nosniff`, no
sandbox and no `Content-Disposition`. Any panel user who can upload a file (a
`FileUpload` field, or the showcase media library, which keeps the client's
extension) can therefore upload an HTML or SVG document whose script runs on the
admin origin for whoever opens its URL — reading CSRF tokens and posting writes
as that admin, across tenants. ADR-0017 made served directories *public* by
decision; it did not decide to serve *active* content same-origin. This plan
makes every served-directory response inert, framework-side, so every app gets
it without touching its uploader.

## Current state

- `crates/argentum-core/src/panel/mod.rs:180-204` — `Panel::serve_dir(path, dir)`
  records `(pattern, dir)` after checking the pattern ends in a catch-all
  (`is_directory_pattern`, ~line 618). `Panel::build` mounts them:

  ```rust
  // panel/mod.rs:549-551
  for (path, dir) in served_dirs {
      builder = builder.serve_dir(route_path(&path), dir);
  }
  ```

  Earlier in `build` (~line 486-493) the pathless frame-ancestors layer is added:

  ```rust
  if let Some(directive) = frame_ancestors {
      builder = builder.layer(headers::FrameAncestors::new(directive));
  }
  ```

- `crates/argentum-core/src/panel/headers.rs` — the exemplar to copy. It defines
  `FrameAncestors`, a `topcoat::router::Layer` with `fn path(&self) -> Option<&Path>`
  (returns `None` = every route) and a `handle` that runs `next` then edits
  response headers, skipping when the response already carries a
  `Content-Security-Policy`:

  ```rust
  impl Layer for FrameAncestors {
      fn path(&self) -> Option<&Path> { None }
      fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
          Box::pin(async move {
              let mut response = next.run(cx, body).await?;
              insert_frame_ancestors(&mut response, &self.directive);
              Ok(response)
          })
      }
  }
  ```

- Topcoat layer selection (`../topcoat/crates/topcoat-router/src/layer.rs:130-144`,
  read-only reference): a layer wraps a route when its `path()` is `None` or a
  prefix of the route path; layers are ordered least- to most-specific, so the
  **pathless `FrameAncestors` runs outermost** and a path-scoped layer runs
  inside it. Consequence: if the new layer sets a `Content-Security-Policy`,
  `FrameAncestors` sees it and leaves it alone — so the new CSP must carry its
  own `frame-ancestors` directive.
- The auth gate layers are path-scoped the same way
  (`AuthGate::path()` in `crates/argentum-core/src/auth.rs:560`), which is the
  second exemplar for a path-scoped layer: it stores a `PathBuf` built with
  `route_path(..)`.
- Tests: `crates/argentum-core/tests/uploads.rs:693-721`
  (`serve_dir_serves_the_upload_directory_through_the_panel`) builds a panel with
  `.serve_dir("/uploads/{*file}", dir)`, writes `cat.png`, and fetches it with the
  file's `get(&router, uri)` / `body_bytes(response)` helpers (lines ~296-310).
  Match that setup for new tests.
- ADR-0017 (`docs/adr/0017-media-uploads.md`) records: served directories are
  public by decision (GH #225). Do not change that.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Unit tests | `cargo test -p argentum-core --lib headers` | all pass |
| Integration | `cargo test -p argentum-core --test it uploads::` | all pass |
| Showcase | `cargo test -p showcase --test it` | all pass |
| Whole workspace | `cargo test --workspace --locked` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo +nightly fmt --all -- --check` | exit 0 |

## Scope

**In scope**:
- `crates/argentum-core/src/panel/headers.rs` (new layer + unit tests)
- `crates/argentum-core/src/panel/mod.rs` (install the layer per served dir; rustdoc of `serve_dir`)
- `crates/argentum-core/tests/uploads.rs` (integration tests)
- `docs/adr/0017-media-uploads.md`, `docs/guide/src/security.md`

**Out of scope**:
- Topcoat itself (`../topcoat`) — do not patch upstream.
- `examples/showcase/src/app.rs` `DirUploader` / `media.rs` — an extension
  allow-list there is a separate, optional follow-up; the framework layer is the fix.
- Changing that served directories are public (ADR-0017 decision).

## Git workflow

- Branch: `advisor/002-serve-dir-inert-content`.
- Squash-merged as one Conventional Commit, e.g.
  `fix(panel): serve uploaded files as inert content (#<issue>)`
  (`docs/dev/COMMITS.md`). Leave `(#<issue>)` if no number was given.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Add the `ServedFileHeaders` layer

In `headers.rs`, add a `pub(crate) struct ServedFileHeaders { path: topcoat::router::PathBuf }`
with `pub(crate) fn new(pattern: &str) -> Self` that stores
`super::route_path(pattern)` (the same full pattern `serve_dir` mounts, e.g.
`/uploads/{*file}` — a route's own path is a prefix of itself, so the layer
wraps exactly that route). Implement `Layer`:

- `path()` → `Some(&self.path)`.
- `handle` → `let mut response = next.run(cx, body).await?;` then call a pure
  function `harden_served_file(&mut response)` and return `Ok(response)`.

`harden_served_file(response: &mut Response)`:
1. Always set `X-Content-Type-Options: nosniff`.
2. Always set `Content-Security-Policy: default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; sandbox; frame-ancestors 'self'`
   — **overwrite** any existing value (a served file never needs a scriptable policy).
3. Read the response `Content-Type` (lowercase, parameters stripped before `;`).
   If it is **not** in the inline allow-list below, set
   `Content-Disposition: attachment` (keep any existing `Content-Disposition`
   only if it already starts with `attachment`).
   Inline allow-list: `image/png`, `image/jpeg`, `image/gif`, `image/webp`,
   `image/avif`, `video/mp4`, `video/webm`, `audio/mpeg`, `audio/ogg`,
   `audio/wav`, `text/plain`. (`image/svg+xml`, `text/html`, `application/pdf`
   and everything else become attachments.)
4. Leave non-2xx responses' bodies alone but still apply 1–2 (harmless).

Document the struct with a `///` comment in the file's style: why it exists (a
served directory shares the admin origin; ADR-0017), and that the policy is
deliberately fixed rather than configurable.

**Verify**: `cargo check -p argentum-core --locked` → exit 0.

### Step 2: Install it for each served directory

In `Panel::build` (`panel/mod.rs` ~line 549), change the loop to:

```rust
for (path, dir) in served_dirs {
    builder = builder
        .layer(headers::ServedFileHeaders::new(&path))
        .serve_dir(route_path(&path), dir);
}
```

Extend `serve_dir`'s rustdoc (lines 180-194) with one sentence: responses carry
`nosniff`, a sandboxing `Content-Security-Policy`, and `Content-Disposition:
attachment` for anything but common raster images, audio/video and plain text,
so an uploaded document cannot run script on the panel's origin.

**Verify**: `cargo test -p argentum-core --test it uploads::` → the existing
`serve_dir_serves_the_upload_directory_through_the_panel` and
`a_served_directory_is_reachable_without_a_session` still pass.

### Step 3: Unit tests for `harden_served_file`

In `headers.rs`'s `#[cfg(test)] mod tests`, build `http::Response`s with a given
`Content-Type` and assert, for each case:
- `image/png` → `nosniff` present, CSP contains `sandbox`, no `Content-Disposition`.
- `image/svg+xml`, `text/html; charset=utf-8`, `application/pdf`,
  missing `Content-Type` → `Content-Disposition` starts with `attachment`.
- a response that already carried `Content-Security-Policy: script-src *`
  ends with the fixed policy (overwritten).

**Verify**: `cargo test -p argentum-core --lib headers` → all pass.

### Step 4: Integration test through the router

In `crates/argentum-core/tests/uploads.rs`, add
`served_active_content_is_inert` modeled on
`serve_dir_serves_the_upload_directory_through_the_panel`: write `cat.png`,
`evil.svg` (`<svg xmlns="http://www.w3.org/2000/svg"/>`) and `evil.html`
(`<p>x</p>`) into the temp dir, build the panel with `.serve_dir("/uploads/{*file}", dir)`,
and assert:
- `/uploads/cat.png` → 200, `x-content-type-options: nosniff`, CSP contains `sandbox`, no `content-disposition`.
- `/uploads/evil.svg` and `/uploads/evil.html` → 200, `content-disposition` starts with `attachment`, CSP contains `sandbox`.
- `/admin/...` panel pages are unaffected: GET the list page of the file's
  existing resource and assert its CSP is still `frame-ancestors 'self'`
  (the panel's default) — proving the layer is scoped to the served path.

**Verify**: `cargo test -p argentum-core --test it uploads::` → all pass including the new test.

### Step 5: Docs

- `docs/adr/0017-media-uploads.md`: after the "public by decision" paragraph, add
  the present-tense rule: a served directory is public **and inert** — every
  response carries `nosniff`, a sandboxing CSP, and `attachment` for anything but
  the inline allow-list; an app that needs to serve active documents mounts them
  on its own origin. Add today's date to the header's `Amended:` list.
- `docs/guide/src/security.md`: one bullet with the same rule.

**Verify**: `cargo test --workspace --locked` → exit 0.

## Test plan

- Unit: `harden_served_file` over content types and a pre-existing CSP (step 3).
- Integration: PNG inline, SVG/HTML attachment, panel pages unaffected (step 4).

## Done criteria

- [ ] `cargo test --workspace --locked` exits 0
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo +nightly fmt --all -- --check` exits 0
- [ ] `grep -n "ServedFileHeaders" crates/argentum-core/src/panel/mod.rs` shows the install in `build`
- [ ] `git status` shows changes only in the in-scope files
- [ ] `plans/README.md` status row updated

## STOP conditions

- The Topcoat layer ordering does not behave as described (e.g. the new test
  shows `FrameAncestors`' `frame-ancestors 'self'` *replacing* the sandbox CSP on
  a served file): report — do not reorder layers by guesswork.
- A path-scoped layer with a catch-all pattern (`/uploads/{*file}`) does not wrap
  the directory route (the headers are missing in step 4). Report the observed
  behavior; do not fall back to a pathless layer that inspects the URI without
  confirming with the operator.
- An existing showcase test asserts a served file opens inline with a type
  outside the allow-list (e.g. a PDF).

## Maintenance notes

- The allow-list is the one knob; widening it (e.g. PDF inline) re-opens risk
  only for types that can run script — PDF in a sandboxed document is safe but
  browsers refuse to render it sandboxed, which is why it downloads.
- Error responses from the directory route (404) return through `Err` and skip
  this layer, like `FrameAncestors` (a separate known issue); they carry no file
  content, so it does not matter here.
- Follow-up (optional, not in this plan): an extension allow-list in the
  showcase `DirUploader`, and documenting "serve uploads from a separate origin"
  in the guide for production deployments.
