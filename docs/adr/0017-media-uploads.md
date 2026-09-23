# Media uploads: an app-level `Uploader` and a clear control

Date: 2026-09-22 — Status: accepted — Amended: 2026-09-22, 2026-09-23

## Decision

**The seam is an app-level trait, not a per-field declaration.**
`Uploader::store(filename, bytes) -> Result<String, String>`, installed once with
`Panel::uploads(uploader)` and found on the app context the way `Db` is — an object store is an app
dependency, and threading it through every `.for(..)` call site would put infrastructure in the schema
declaration. The public trait returns `impl Future` (the house style, no `async_trait`), and a private
dyn-compatible shim holds it so `Panel` does not become generic over the app's store. The request side
is the framework's already: the 10 MiB body cap, the multipart stream, and filename sanitization to a
basename.

**Bytes are buffered, not streamed to the store,** because the body cap already bounds memory; the
buffer is taken only when an uploader is installed, so with none the parser keeps draining and
discarding. **A refusal is user input, not infrastructure:** `Err(reason)` renders
`"<Label> could not be uploaded: <reason>"` against the field and re-renders the form with the
submitted values — the framework owns the sentence, the uploader owns the reason, and the reason is
printed to the user, so never a driver message or a path. **The framework stores the returned string
verbatim and renders it verbatim:** the field still binds a `String`, the record fn's contract is
unchanged, and a stored value renders as a link to the file — on the edit form and on the detail
page. The field reads no extension and owns no image pipeline, so it neither previews a path nor
guesses a URL convention (GH #242).

**The primitive stops at the file input.** A thumbnail in the stored row, an × that clears the input
without JavaScript, drag-and-drop and upload progress are media-library work, tracked by GH #248:
they need the app's own media table and its own assets, and a generic `String`-bound field is the
wrong place to guess them.

**The clear control is a declared transport key.** `FileUpload` renders a `clear_<field>` checkbox
whenever a value is stored, alongside the hint that an empty file input keeps what is there.
Strip-before-record-fn (GH #148) is unchanged, so a generic `Resource` impl still cannot be handed the
flag as a write. Clearing does not waive `required`: the value is empty, the ordinary required error
answers, and the record keeps its file — a resource that may lose its file declares `.optional()`.

**`Panel::serve_dir(path, dir)` mounts an app-owned directory** on the panel's router, the app's only
way to add a route the framework does not own. The passthrough is deliberately narrow (upstream's
`serve_dir`, path pattern included) rather than a general route hook, and the path is not
panel-relative: a served directory holds files a record points at, not panel pages, and its URLs must
not move when the panel is mounted elsewhere. A served directory is **public by decision** (GH #225):
the auth gate installs exactly two layers — the panel prefix and `/_topcoat/runtime` (ADR-0013) — so a
directory mounted outside both is ungated by construction. Public media is a legitimate shape, and
gating a served directory remains a possible future option; the rule for apps is that a directory meant
to be private is mounted behind the app's own gate, never assumed private from the mount path.
`a_served_directory_is_reachable_without_a_session` in `crates/argentum-core/tests/uploads.rs` pins
the anonymous case.

Uploads run **before** the write transaction and outside it: an upload is a side effect in another
system, and a rolled-back transaction must not have to undo it. A file stored for a form that then
fails validation is unreferenced, not wrong, and a store with a real write cost wants its own janitor.

## Consequences

- An app that never installs an `Uploader` is unaffected: the sanitized basename is stored, and the
  stored value renders as a link to the file.
- A cleared upload empties the stored value, not the bytes: the framework cannot delete from a store
  it does not know. An app that wants the bytes gone acts on the empty value its record fn receives.
- A rejected store drops the submitted value rather than blanking it, which is why the edit handler
  stores before it backfills: a re-rendered edit shows the file still on disk, and a re-rendered create
  shows an empty field beside the reason.
- The showcase demonstrates the whole path: `DirUploader` writes into a served directory, the record
  stores the returned URL, and the URL fetches the bytes back (GH #188).
- `argentum-core` enables topcoat's `fs` feature, which upstream's directory route lives behind, and
  both lockfiles carry the crates it pulls. Image handling (thumbnails, transcoding, dimensions) and
  storage drivers stay out of scope: the field links a stored path, and the trait is the seam for the
  bytes.
