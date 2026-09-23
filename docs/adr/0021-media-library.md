# Media library: a polymorphic `medias` table in the showcase

Date: 2026-09-23 — Status: accepted

## Decision

**The library is the app's, and its rows are polymorphic.** `argentum-core` keeps the seam
ADR-0017 drew — a `FileUpload` binds a `String`, the bytes go to the app's `Uploader`, and the
stored value renders as a link — and the media library is the showcase's:
`examples/showcase/src/models.rs` declares `MediaAsset`, `#[table = "medias"]`, one row per stored
file with the tenant that uploaded it, the `path` the `Uploader` returned, the client's
`filename`, a `kind`, and a timestamp. `examples/showcase/src/media.rs` is the page that fills and
renders it.

**The owner is an `owner_type`/`owner_id` pair, and that is the tradeoff.** `owner_type` is `"post"`
or `"user"`, and `owner_id` is that record's key. The typed ORM cannot express the relation: a
`#[belongs_to]` names one target model and one key column, so the pair carries no foreign key,
`MediaAsset` declares no relation, and neither `Post` nor `User` gets a `#[has_many]`. Integrity is
the app's, in three places:

- `MediaOwner` (`Post(Uuid)` / `User(Uuid)`) is how the app recovers the type the pair lost, and
  `media_for_owner(db, owner)` is the whole relation — one equality filter on the pair.
- The upload handler refuses an owner that does not resolve before it writes: a post through the
  tenant-scoped `scoped_query::<PostResource>` the panel uses (GH #223), a user through the users
  table, which is global in this app.
- Deleting an owner leaves its media rows dangling. The page renders them under `Post · (deleted)`
  or `User · (deleted)` rather than hiding a row nothing cascaded.

The alternatives are worse for this shape. **Nullable per-owner foreign keys** (`post_id`,
`user_id`, exactly one set) carry real integrity, but the table grows a column per owner kind and
every read still decides which one is set — the same app-side check plus a nullable column per
owner. **A Post-only media model** is simplest and needs no pair, but it cannot hold a user's
avatar, the second owner the showcase demonstrates.

**The vocabulary splits three ways.** `Media` stays the embedded enum on `Post` (GH #185): an
image/video *description* flattened into the post's own columns, with no bytes and no owner.
`Attachment` stays the label over that value's controls in the post form — a heading, not a term.
The new row is a **`MediaAsset`**: a file in the library, with bytes behind its `path` and an owner.
The table keeps the issue's name (`medias`) through `#[table = "medias"]`, because the model name is
what would collide. `CONTEXT.md` carries the same split.

**The library is a page, not a `Resource`.** `GET /admin/media` renders the tenant's rows and the
upload form; `POST /admin/media` parses the multipart body, stores the bytes through the app's own
`Uploader` (the `DirUploader` the panel installs, GH #188), and writes the row. Neither seam fits:
a `Table` column projects a `String`, so it cannot render a thumbnail, and the `Schema` tree has no
node for a stored file's preview. The page parses its own form for the same reason, which is also
why it verifies the CSRF token itself (`csrf::verify`, GH #99) and reduces the client filename to a
basename before the store sees it — the store takes the basename again rather than trusting a
caller to have done it (GH #90).

**The rich upload UX is the app's, and its no-JS fallback is a reset button.** The page renders the
file input, a preview region (`data-media-preview`), and an × (`data-media-clear`) that is a
`type="reset"` control: a browser empties the file input by resetting the form, with no script at
all. `examples/showcase/assets/media.js` adds the preview — an `<img>` from an object URL for an
`image/*` file, the file's name otherwise, the URL revoked when the preview is replaced or cleared
— and drops the preview when the reset control is clicked. It never cancels that click: the reset
is what empties the input, and the script owns only what it drew.

The script is the **app's asset**, declared as `MEDIA_JS` and linked `defer`red by the page, not a
tenth shell asset: ADR-0014's ownership and all-load rules govern the assets `argentum-ui` ships
and `render_document` emits, and a shell asset would load media-widget code into every admin
document of every app for a widget one page renders. The app owns its script the way it owns its
`styles.css` (ADR-0006).

**A thumbnail is decided by content type, not by a suffix.** A row's `kind` is `"image"` when the
uploaded part's `Content-Type` is `image/*`, and `"file"` otherwise. The framework deleted its
extension check and its preview (GH #242) because a filename is not a content type; classifying is
the app's, and what it classifies is what the browser said the bytes are. An image row renders
`<img>`, anything else a link, through one view the media page and the public blog's post page
share. The library generates no derivative: the thumbnail is the stored image sized down in the
page, so nothing here reads or rewrites the bytes after the store returns.

## Consequences

- `examples/showcase/tests/media_check.rs` covers the upload path (a row tied to its owner, the
  store's URL fetched back, a basename that cannot climb out of the served directory), the
  thumbnail/link split, the reset control, and the refusals (a dangling owner, a missing CSRF
  token, a tenantless request). `examples/showcase/assets/media.test.js` covers the script, and
  CI's `assets` job runs it.
- The library lists one tenant's rows — the uploader's. A user-owned row is not scoped by its owner,
  because the showcase's users are global; the row's own `tenant_id` is what the page filters on.
- The list is one unpaginated query: a `Table` paginates, and this page is not one. A library that
  outgrows a page wants its own loader and pager.
- The panel's sidebar is derived from its `Resource`s (ADR-0008), so a hand-written page has no
  navigation entry; the library is reached at `/admin/media`.
- The page links its script only when the router carries an asset bundle, so a test router renders
  the markup without one. A bundle built before `media.js` existed does not carry it and the page
  panics on render, like any other asset the bundle is missing; `topcoat dev` re-bundles.
- Deleting a media row does not delete its bytes: the store is the app's, and a janitor for
  unreferenced files stays the app's business (ADR-0017).
