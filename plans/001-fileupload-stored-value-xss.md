# Plan 001: Close the stored-XSS path through `FileUpload` values

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat cbb738a8..HEAD -- crates/argentum-core/src/schema/fields/file_upload.rs crates/argentum-core/src/panel/forms.rs crates/argentum-core/src/upload.rs crates/argentum-core/tests/uploads.rs examples/showcase/tests/ docs/adr/0017-media-uploads.md docs/guide/src/security.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M (step A alone is S)
- **Risk**: MED — step B changes which submissions a `FileUpload` field accepts and rewrites ~12 showcase test call sites
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `cbb738a8`, 2026-09-24

## Why this matters

A `FileUpload` field's value is meant to be "the string the installed `Uploader`
returned" (ADR-0017). Today any panel user with create/update rights can instead
submit an arbitrary text value for that field — a url-encoded body, or a multipart
part with no `filename` — and it is stored verbatim, skipping the uploader. That
value is then rendered verbatim into `<a href=…>` on the edit form ("Current: …")
and on the detail page, and neither Topcoat's `view!` nor the panel's headers
restrict the URL scheme. A stored `javascript:` URL therefore runs in another
admin's session (same origin as the panel, so it can read CSRF tokens and post
writes) when they click the link. Two independent defenses land here: (A) never
render a non-http(s)/non-rooted value as a link; (B) never accept a client-typed
text value for a declared `FileUpload` field.

## Current state

- `crates/argentum-core/src/schema/fields/file_upload.rs` — the `FileUpload`
  field. `stored_upload_row` (edit form, ~line 233) and `stored_upload_value`
  (detail page, ~line 257) render the stored string as `href`:

  ```rust
  // file_upload.rs:233-246
  fn stored_upload_row<'a>(cx: &'a Cx, path: String) -> BoxView<'a> {
      let href = path.clone();
      let text = path.clone();
      view! {
          cx =>
          <div class="text-xs text-muted-foreground" data-file-current=(path)>
              "Current: "
              <a class="font-medium text-foreground underline" href=(href)>(text)</a>
          </div>
      }
      .boxed()
  }
  ```

  ```rust
  // file_upload.rs:257-272
  fn stored_upload_value<'a>(cx: &'a Cx, label: &str, value: Option<&str>) -> Result<BoxView<'a>> {
      let Some(path) = stored_path(value) else {
          return render_value(cx, label, value, ValueKind::Machine);
      };
      let href = path.clone();
      let link = view! {
          cx =>
          <a
              class="text-sm font-mono break-all whitespace-pre-wrap underline"
              href=(href)
          >
              (path)
          </a>
      }
      .boxed();
      render_value_view(cx, label, link)
  }
  ```

- `crates/argentum-core/src/panel/forms.rs` — form decoding and the create/edit
  handlers. `FormParts` (~line 34) holds `values` and `files`. In
  `parse_multipart_values` (~line 105), a part **with** a filename goes through
  `sanitize_filename` and (when an uploader is installed) is staged in `files`;
  a part **without** a filename is treated as text:

  ```rust
  // forms.rs:162-168 (the `None` arm of `match filename`)
  None => {
      let text = field.text().await?;
      count_form_bytes(&mut bytes_seen, text.len())?;
      out.values.insert(name, text);
  }
  ```

  The url-encoded path (`parse_form_body`, ~line 76) puts every pair in
  `values` with `files` empty.

- `crates/argentum-core/src/upload.rs:139-142` — `store_uploads` only replaces
  values for fields present in `files`, so a text value for the same name
  passes through untouched:

  ```rust
  for (name, upload) in schema.file_uploads() {
      let Some(staged) = files.get(&name) else {
          continue;
      };
  ```

- Create handler (`forms.rs` ~line 600): `parse_form_body` → `csrf::verify` →
  `reject_unknown_form_keys` → `let FormParts { mut values, files } = parts;` →
  `store_uploads` → `strip_transport_keys` → validate → write.
- Edit handler (`forms.rs` ~line 716): same, plus a backfill loop after
  `store_uploads` that restores the record's stored value when the submitted
  value is empty and `clear_<name>` is not set:

  ```rust
  for name in schema.file_uploads().keys() {
      let cleared = values
          .get(&format!("clear_{name}"))
          .is_some_and(|v| truthy(v));
      let empty = values
          .get(name)
          .map(|v| v.trim().is_empty())
          .unwrap_or(true);
      if !cleared && empty && current.get(name).is_some_and(|v| !v.trim().is_empty()) {
          values.insert(name.clone(), current[name].clone());
      }
  }
  ```

- `schema.file_uploads()` (`crates/argentum-core/src/schema/mod.rs` ~line 277)
  returns a `HashMap<String, FileUpload>` keyed by field name.
- `FileUpload` is required by default (the showcase Post form's
  `FileUpload::r#for(Post::fields().image_path())`, `examples/showcase/src/app.rs:748`).

Conventions: comments explain *why* and cite the GH issue; unit tests live in the
`#[cfg(test)] mod tests` at the bottom of the same file (see the existing
`file_upload.rs` tests from ~line 285, which use `tag_with`/`attributes_of` from
`super::test_support`). Docs describe current behavior only — never "used to" or
"previously" (`docs/dev/PROSE.md`). `docs/dev/TESTING.md`: pin behavior (status
codes, DB state, attribute presence), not copy.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Unit tests (field) | `cargo test -p argentum-core --lib file_upload` | all pass |
| Core integration | `cargo test -p argentum-core --test it uploads::` | all pass |
| Showcase suite | `cargo test -p showcase --test it` | all pass |
| Whole workspace | `cargo test --workspace --locked` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo +nightly fmt --all -- --check` | exit 0 |
| view! format | `topcoat fmt && git diff --exit-code` | exit 0 (requires the locked-rev CLI, see below) |

Install the locked-rev `topcoat` CLI if `topcoat fmt` is missing or a different
version (from `AGENTS.md`):

```sh
REV=$(grep -A 2 '^name = "topcoat"$' Cargo.lock | grep -o '#[0-9a-f]\{40\}' | head -1 | cut -c2-)
cargo install --git https://github.com/tokio-rs/topcoat --rev "$REV" topcoat-cli --locked
```

Never pipe a command through `| tail` when you need its exit code.

## Scope

**In scope**:
- `crates/argentum-core/src/schema/fields/file_upload.rs`
- `crates/argentum-core/src/panel/forms.rs`
- `crates/argentum-core/tests/uploads.rs`
- `examples/showcase/tests/common/mod.rs` (add one helper)
- `examples/showcase/tests/{relation_check,file_repeater_check,edit_check,tenancy_check}.rs` (migrate call sites, step B)
- `docs/adr/0017-media-uploads.md`, `docs/guide/src/security.md`, `docs/guide/src/forms.md` (one-paragraph updates)

**Out of scope**:
- `crates/argentum-core/src/upload.rs` — the `Uploader` trait and `store_uploads` stay as they are.
- `crates/argentum-ui/src/components/primitives/` — vendored, never hand-edited.
- The showcase media library (`examples/showcase/src/media.rs`) — it does not use `FileUpload`.
- Serving uploaded HTML/SVG (that is plan 002).

## Git workflow

- Branch: `advisor/001-fileupload-stored-value-xss`.
- Commit as you like on the branch; the branch is squash-merged into `master` as one
  Conventional Commit: `fix(schema)!: accept a FileUpload value only from a file part (#<issue>)`
  (`docs/dev/COMMITS.md`; the `!` is because step B refuses input that used to be accepted).
  If no issue number was given to you, leave `(#<issue>)` for the maintainer.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step A1: Render a stored value as a link only when it is a safe URL

In `file_upload.rs`, add a private helper next to `stored_path`:

```rust
/// Whether a stored value may become an `href` (GH #<issue>): a rooted path
/// (`/uploads/x.png`, not the scheme-relative `//host`) or an absolute
/// `http(s)` URL. Anything else — a bare basename, `javascript:`, `data:` —
/// renders as text: the framework stores what it is handed, so the render is
/// where a scheme is refused.
fn is_linkable(path: &str) -> bool {
    let lower = path.trim_start().to_ascii_lowercase();
    (lower.starts_with('/') && !lower.starts_with("//"))
        || lower.starts_with("https://")
        || lower.starts_with("http://")
}
```

Change `stored_upload_row` and `stored_upload_value` so that when
`!is_linkable(&path)` they render the same text **without** an `<a>`:
- edit row: keep the `<div … data-file-current=(path)>` wrapper and the
  `"Current: "` text; render `<span class="font-medium text-foreground">(text)</span>`
  instead of the `<a>`.
- detail value: fall back to `render_value(cx, label, Some(&path), ValueKind::Machine)`
  (the same call already used when nothing is stored).

Note: a backslash or control char before the scheme is not a concern because the
check lowercases and trims only leading whitespace, and anything not starting
with `/`, `http://`, `https://` is not linked.

**Verify**: `cargo test -p argentum-core --lib file_upload` → all existing tests pass
(they use rooted `/uploads/...` values and a `/media/photo.JPG?token=abc` value, which stay links).

### Step A2: Unit-test the render guard

In the `#[cfg(test)] mod tests` of `file_upload.rs`, add one test modeled on the
existing test around line 440-475 that renders a stored value in both `Mode::Edit`
(the form) and `Mode::View` (detail). For each of `javascript:alert(1)`,
`JavaScript:alert(1)`, `data:text/html,x`, `//evil.example/x.png`, `report.pdf`,
assert the rendered HTML contains no `href=`; for `/uploads/a.png` and
`https://cdn.example/a.png` assert an `<a` carrying `href="<value>"` is present
(use `tag_with(&html, "href=\"…\"").starts_with("<a")` like the existing tests).

**Verify**: `cargo test -p argentum-core --lib file_upload` → all pass, including the new test.

Run `topcoat fmt && git diff --exit-code -- crates/argentum-core/src/schema/fields/file_upload.rs`;
if `topcoat fmt` changed the file, keep its changes (they are the canonical layout) and re-run the tests.

### Step B1: Record which fields arrived as file parts

In `forms.rs`, add a field to `FormParts`:

```rust
/// Field names that arrived as a multipart part carrying a `filename`
/// (chosen or empty). Only these may set a `FileUpload` value: a text part or
/// a url-encoded pair under the same name is client-typed, not an upload.
pub(crate) file_part_names: std::collections::HashSet<String>,
```

- Initialize it empty in both constructors (`parse_multipart_values` and the
  url-encoded return in `parse_form_body`).
- In `parse_multipart_values`, insert `name.clone()` into it in **both**
  `Some(f) if !f.is_empty()` and `Some(_)` arms (a file input with no file chosen
  still sends a part with `filename=""`; it means "keep" on edit and must not be
  treated as forged). Do not insert in the `None` arm.
- Update every destructuring of `FormParts` (create and edit handlers, and
  `auth.rs` which reads `.values` only — field access is unaffected) so it compiles.

**Verify**: `cargo check -p argentum-core --locked` → exit 0.

### Step B2: Drop client-typed values for declared uploads

Add a helper in `forms.rs` near `strip_transport_keys`:

```rust
/// Drop any value a declared `FileUpload` received from something other than a
/// file part (GH #<issue>). The field's value is the uploader's answer, the
/// stored value (edit backfill), or empty (clear) — never text the client typed,
/// which would reach the record and render as the file's link.
fn drop_client_typed_uploads(
    schema: &crate::schema::Schema,
    file_part_names: &std::collections::HashSet<String>,
    values: &mut HashMap<String, String>,
) {
    for name in schema.file_uploads().keys() {
        if !file_part_names.contains(name) {
            values.remove(name);
        }
    }
}
```

Call it in the create handler and the edit handler **immediately after**
`let FormParts { mut values, files, file_part_names } = parts;` and **before**
`store_uploads`. On edit, the existing backfill loop then restores the stored
value (the value is now absent → `empty == true`), so an untouched or forged
edit keeps the stored file; `clear_<name>=1` still clears it. On create, the
field is empty and `required` answers.

**Verify**: `cargo test -p argentum-core --test it uploads::` → all pass. If a test
there posts a `FileUpload` value url-encoded and asserts it is stored, it pins the
behavior this plan removes: convert it to a multipart file part (the file already
has multipart builders — search it for `Content-Disposition: form-data`).

### Step B3: Integration tests for the forged value

In `crates/argentum-core/tests/uploads.rs`, add two tests using the file's
existing `Doc`/`DocResource` fixtures and request helpers (copy the setup of the
nearest existing create/edit test in that file):
1. `a_text_value_for_a_file_upload_is_not_stored_on_create`: POST a url-encoded
   create with `cover=javascript:alert(1)` (and valid other fields + CSRF).
   Assert the response re-renders the form (status 200) and **no** `Doc` row was
   created (the required `cover` is empty).
2. `a_text_value_for_a_file_upload_keeps_the_stored_file_on_edit`: seed a `Doc`
   whose `cover` is `/uploads/old.png`; POST an edit with `cover=javascript:alert(1)`
   as a **multipart text part** (no `filename`). Assert a redirect and that the row's
   `cover` is still `/uploads/old.png`.

**Verify**: `cargo test -p argentum-core --test it uploads::` → all pass including 2 new tests.

### Step B4: Migrate the showcase call sites that post `image_path` as text

The showcase `Post` form has a required `FileUpload` on `image_path`. These call
sites post `image_path=<text>` url-encoded (found with
`grep -n "image_path=" examples/showcase/tests/*.rs`):

- creates that expect a row to be created — now they get "required":
  `relation_check.rs` (~lines 86, 121, 321, 359), `file_repeater_check.rs`
  (~lines 123, 168), `edit_check.rs` (~line 531), `tenancy_check.rs` (~line 239).
- `file_repeater_check.rs:142` asserts `post.image_path == "/tmp/valid.jpg"`, a
  client-typed value — this test pins the removed behavior.
- edits posting `image_path={post.image_path}` or `image_path=` keep working
  unchanged (the backfill restores the stored value) — leave them.
- `tenancy_check.rs:156` and `relation_check.rs:50` expect a refusal (403/validation)
  for other reasons — run them first; leave them if they still pass.

Add a helper to `examples/showcase/tests/common/mod.rs`:

```rust
/// A multipart body: text fields, then one file part per `(field, filename, bytes)`.
pub fn multipart_body(boundary: &str, fields: &[(&str, &str)], files: &[(&str, &str, &str)]) -> String
```

that emits `--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n`
per field, `--{boundary}\r\nContent-Disposition: form-data; name="{k}"; filename="{f}"\r\nContent-Type: application/octet-stream\r\n\r\n{bytes}\r\n`
per file, and a closing `--{boundary}--\r\n` (copy the exact framing from
`examples/showcase/tests/upload_check.rs:40-50`). Convert each failing create call
site to `client.csrf(&csrf).post_multipart(uri, boundary, multipart_body(..))`
with the other fields as text parts and `image_path` as a file part
(e.g. `("image_path", "valid.jpg", "FAKEBYTES")`). The showcase router installs
an uploader (`router()` → `upload_dir()`), so the stored value becomes the
uploader's path: replace `assert_eq!(post.image_path, "/tmp/valid.jpg")` with an
assertion that it ends with `valid.jpg` and starts with `/` (the uploader's URL
prefix), matching `upload_check.rs:84`.

**Verify**: `cargo test -p showcase --test it` → all pass.

### Step C: Docs

- `docs/adr/0017-media-uploads.md`: in the Decision, next to "The framework stores
  the returned string verbatim and renders it verbatim", state the two rules in
  present tense: a `FileUpload` value comes only from a file part (the uploader's
  answer, or the sanitized basename with no uploader), the stored value on an
  untouched edit, or empty on clear; a stored value renders as a link only when
  it is rooted (`/…`) or `http(s)://…`, otherwise as text. Add today's date to
  the header's `Amended:` list.
- `docs/guide/src/security.md`: add one bullet stating the same two rules.
- `docs/guide/src/forms.md`: if it describes how `FileUpload` values are
  submitted or rendered, align that sentence.

**Verify**: `grep -n "used to\|previously" docs/adr/0017-media-uploads.md docs/guide/src/security.md` → no new matches from your edits.

## Test plan

- Unit (step A2): render guard over scheme variants, both modes.
- Integration (step B3): forged url-encoded create, forged multipart text part on edit.
- Regression: the whole showcase suite with migrated call sites (step B4).

## Done criteria

- [ ] `cargo test --workspace --locked` exits 0
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo +nightly fmt --all -- --check` exits 0
- [ ] `topcoat fmt && git diff --exit-code` exits 0 (locked-rev CLI)
- [ ] `grep -n "fn is_linkable\|fn drop_client_typed_uploads" crates/argentum-core/src/schema/fields/file_upload.rs crates/argentum-core/src/panel/forms.rs` shows both helpers
- [ ] `git status` shows changes only in the in-scope files
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts above do not match the live code.
- More than the listed showcase call sites fail in step B4, or a failing test
  asserts something other than "the typed text was stored" / "the row was created"
  (it may pin a behavior this plan did not anticipate).
- Browsers are found to submit a `FileUpload` field *without* a `filename` part
  in some flow the showcase uses (e.g. a JS-built `FormData`): step B would then
  blank legitimate edits. Report the flow.
- `auth.rs` or any other caller relies on a `FileUpload` value arriving as text.

## Maintenance notes

- Any future field type that renders a stored string as `href` or `src` must go
  through `is_linkable` (or a shared successor) — consider moving it to
  `schema/fields/mod.rs` when a second caller appears.
- If an app wants to set a file path from a text box (e.g. "paste a URL"), that
  is a different field type (a `TextInput` with URL validation), not a relaxation
  of `FileUpload`.
- Reviewers: check that the edit backfill still runs *after* `drop_client_typed_uploads`,
  and that the empty-filename part still counts as a file part.
- Plan 002 (serving uploaded active content) is the complementary defense for
  values the uploader itself returns.
