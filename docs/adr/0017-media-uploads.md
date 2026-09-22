# Media uploads: an app-level `Uploader`, a clear control, and a preview

Date: 2026-09-22 — Status: accepted — Supersedes: none

## Context

`FileUpload` binds a `String` column and renders a file input, and the form
parser decodes `multipart/form-data` — but it stored the **sanitized client
filename** and discarded the bytes (`GH #73`: "binary/file-asset handling is
future work"). Three consequences accumulated: an upload was cosmetic in a real
deployment, the server honoured `clear_<field>=1` while nothing rendered a
control that sent it (`GH #90`), and an edit showed the stored path as text but
not the file, so a user could not tell whether the stored value was right
without leaving the panel.

The request side is entirely the framework's already: the body cap (10 MiB,
`GH #90`), the multipart stream, and filename sanitization to a basename
(`GH #90`, `GH #149`). What was missing is the one decision the framework
cannot make — *where the bytes live*.

## Decision

**1. The seam is an app-level trait, not a per-field declaration.**
`Uploader::store(filename, bytes) -> Result<String, String>`, installed once
with `Panel::uploads(uploader)` and found on the app context the way `Db` is.
An object store is an app dependency; threading it through every `.for(..)`
call site would put infrastructure in the schema declaration. The public trait
returns `impl Future` (the house style — no `async_trait`), and a private
dyn-compatible shim holds it so `Panel` does not become generic over the app's
store.

**2. Bytes are buffered, not streamed to the store.** The request body is
capped at 10 MiB, so the memory bound is one the deployment already accepted;
a streaming signature would be a bigger seam with no consumer. The buffer is
taken **only when an uploader is installed**: with none, the parser keeps
draining and discarding, so the default path's memory profile is unchanged.

**3. Refusal is user input, not infrastructure.** `Err(reason)` renders
`"<Label> could not be uploaded: <reason>"` against the field and re-renders
the form with the submitted values. The framework owns the sentence and the
uploader owns the reason, so every field error stays label-anchored
(`"<Label> is required"`, `"<Label> has already been taken"`) and an uploader
never has to know a label. The reason is printed to the user: never a driver
message, a path, or anything else the deployment would rather not show.

**4. The framework stores the returned string verbatim and renders it
verbatim.** No URL convention is invented: the field still binds a `String`,
the record fn's contract is unchanged, and the preview is a suffix check — an
image extension renders an `<img>` (the full image with a size cap; thumbnails
imply an image pipeline the framework does not have), anything else renders a
link to the file.

**5. The clear control is a declared transport key.** `FileUpload` renders a
`clear_<field>` checkbox whenever a value is stored, alongside the hint that an
empty file input keeps what is there. Strip-before-record-fn (`GH #148`) is
unchanged, so a generic `Resource` impl still cannot be handed the flag as a
write. Clearing does **not** waive `required`: the value is empty, the ordinary
required error answers, and the record keeps its file. That is the mainstream
behaviour (Django's clearable file input is the same shape) and it keeps the
app in control — a resource that may lose its file declares `.optional()`.

**6. `Panel::serve_dir(path, dir)` mounts an app-owned directory.** The Panel
owns the `Router`, so an app whose uploader writes files had nowhere to serve
them from; the demo that proves the seam needs exactly that. The passthrough is
deliberately narrow (upstream's `serve_dir`, path pattern included) rather than
a general route hook, and the path is **not** panel-relative: a served
directory holds files a record points at, not panel pages, and its URLs must
not move when the panel is mounted elsewhere.

Uploads run **before** the write transaction and outside it: an upload is a
side effect in another system, and a rolled-back transaction must not have to
undo it. The consequence is documented rather than engineered away — a file
stored for a form that then fails validation is unreferenced, not wrong, and a
store with a real write cost wants its own janitor.

Considered: (A) storing bytes in the database (rejected: the framework would
be picking a storage model, and blobs in the row are usually the wrong
default), (B) a per-field `.store(..)` declaration (rejected: it puts an
app-level dependency in every schema that has an upload and gives no single
place to install a driver), (C) reporting a refusal as an error page (rejected:
a rejected upload is the user's input going wrong, and reporting it as
infrastructure failure is the bug GH #174 fixed elsewhere), (D) a first-class
"clear" field (rejected: the flag is transport, not data — it decides whether
the stored value survives the submit and must never reach a record fn, which
the existing declared-key vocabulary makes structural rather than
conventional), (E) a general `Panel::route(impl Route)` hook (rejected for now:
wider API for one caller, and topcoat's `Route` is not dyn-compatible so it
would need a boxed registrar to hold — add it when a second caller exists).

## Consequences

- An app that never installs an `Uploader` is unaffected: the sanitized
  basename is stored, exactly as before — and for such an app the stored value
  now renders as a link or an image, which is honest about a path the framework
  cannot resolve but does change what an old row looks like.
- A cleared upload empties the **stored value**, not the bytes: the framework
  cannot delete from a store it does not know. An app that wants the bytes gone
  too acts on the empty value its record fn receives.
- A rejected store drops the submitted value rather than blanking it, which is
  why the edit handler stores before it backfills: a re-rendered edit then
  shows the file that is still on disk, and a re-rendered create shows an empty
  field beside the reason instead of the client's filename dressed up as a
  stored file.
- The showcase demonstrates the whole path — `DirUploader` writes into a
  served directory, the record stores the returned URL, and the URL fetches the
  bytes back (GH #188's acceptance).
- `argentum-core` now enables topcoat's `fs` feature, which is what upstream's
  directory route lives behind, and both lockfiles carry the crates it pulls.
- Image handling beyond a preview (transcoding, thumbnails, dimensions) stays
  out of scope, as do storage drivers: the trait is the seam, drivers are the
  app's business.

**Status 2026-09-22 (GH #225): served directories are public, by decision.**
Decision 6 above fixes the mount shape but never says who may read it. The
answer is everyone: the auth gate installs exactly two layers — the panel prefix
and `/_topcoat/runtime` (ADR-0013) — so a directory mounted outside both is
ungated by construction and its URLs are served to whoever asks. That is kept
deliberately rather than inherited: public media is a legitimate shape (an
upload store's output is a record's file, and the panel's own pages already
publish those URLs), and gating a served directory remains a possible future
option, not a bug fix. The rule this leaves for apps: **a directory meant to be
private is expressed explicitly** — mount it behind the app's own gate — never
assumed from the mount path. `a_served_directory_is_reachable_without_a_session`
in `crates/argentum-core/tests/uploads.rs` pins the anonymous case, because the
older `serve_dir` test runs with `Auth::disabled()` and the showcase's
fetch-back test uses a logged-in client, so neither exercised it.
