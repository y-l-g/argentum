# Media library: a polymorphic `medias` table in the showcase

Date: 2026-09-23 — Status: accepted — Amended: none

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
`Uploader` (the same `DirUploader` the app gives `Panel::uploads`, GH #188), and writes the row.
Neither seam fits: a `Table` column projects a `String`, so it cannot render a thumbnail, and the
`Schema` tree has no node for a stored file's preview. The page parses its own form for the same
reason, which is also why it verifies the CSRF token itself (`csrf::verify`, GH #99) and reduces the
client filename to a basename before the store sees it. The store applies that same rule itself
rather than trusting a caller (GH #90), so the row's `filename` is exactly the name the store
writes, bar its `{uuid}-` prefix, and the length cap leaves room for that prefix inside the
255-byte filename limit.

**The store's return value is a URL, so it is percent-encoded.** The framework renders what the
store returns verbatim as the file's link (GH #242), and a client filename is arbitrary bytes: an
unencoded `cover #1.png` becomes `cover ` plus a fragment, a `%22` — what Chrome sends for a quote —
decodes back to a quote the file on disk does not carry, and a trailing space disappears in URL
parsing. `DirUploader` encodes the name as one path segment (RFC 3986 unreserved kept, everything
else `%XX`), so every name a browser can send resolves back to its bytes. The media library's page
and the framework's `FileUpload` share this store, so both links are fixed together.

**The rich upload UX is the app's, and its no-JS fallback is a reset button.** The page renders the
file input, a preview region (`data-media-preview`), and an × (`data-media-clear`) that is a
`type="reset"` control. With no script the browser resets the form: the file input empties, which is
the fallback, and so does everything else in the form — the owner picker included, since a reset is
what the markup declares. `examples/showcase/assets/media.js` narrows that: it empties the file
input and the preview itself and cancels the reset, so a file clear keeps the owner the user picked.
The script draws the preview in the first place — an `<img>` from an object URL for an `image/*`
file, the file's name otherwise, the URL revoked when the preview is replaced or cleared — and a
form with no file input in reach keeps the browser's reset rather than swallowing the click.

The script is the **app's asset**, declared as `MEDIA_JS` and linked `defer`red by the page rather
than added to the shell's set. ADR-0014 owns the scripts `argentum-ui` ships — the document emits
them, and no component emits its own — and this is the app's own script on the app's own page: a
tenth shell asset would load media-widget code into every admin document of every app (ADR-0014's
all-load policy) for a widget one page renders, and the framework offers no seam for an app-supplied
shell script at all. The app owns its script the way it owns its `styles.css` (ADR-0006).

**A thumbnail is decided by content type, not by a suffix.** A row's `kind` is `"image"` when the
uploaded part's `Content-Type` is `image/*`, and `"file"` otherwise. The framework deleted its
extension check and its preview (GH #242) because a filename is not a content type; classifying is
the app's, and what it classifies is what the browser said the bytes are. An image row renders
`<img>`, anything else a link, through one view the media page and the public blog's post page
share. The library generates no derivative: the thumbnail is the stored image sized down in the
page, so nothing here reads or rewrites the bytes after the store returns.

## Consequences

- `examples/showcase/tests/media_check.rs` covers the upload path (a row tied to its owner, the
  store's URL fetched back, a basename that cannot climb out of the served directory, filenames a
  URL would otherwise break — `#`, a space, `%22`, `%`, a trailing space), one owner's rows and not
  another's, the thumbnail/link split, the reset control, and the refusals (a dangling owner, a
  missing CSRF token, a tenantless request). `examples/showcase/assets/media.test.js` covers the
  script, and CI's `assets` job runs it.
- The library lists one tenant's rows — the uploader's. A user-owned row is not scoped by its owner,
  because the showcase's users are global; the row's own `tenant_id` is what the page filters on.
- The list is one unpaginated query: a `Table` paginates, and this page is not one. A library that
  outgrows a page wants its own loader and pager.
- The panel's sidebar is derived from its `Resource`s (ADR-0008), so a hand-written page has no
  navigation entry; the library is reached at `/admin/media`.
- The page builds the app's store from the same `upload_dir()` the panel is configured with, because
  the `Uploader` `Panel::uploads` installs lives on the app context for the framework's form parser
  and is not readable from a page. An app whose store is configured elsewhere gives the page the same
  store it gives the panel.
- The page links its script only when the router carries an asset bundle, so a test router renders
  the markup without one. A bundle built before `media.js` existed does not carry it and the page
  panics on render, like any other asset the bundle is missing; `topcoat dev` re-bundles.
- Deleting a media row does not delete its bytes: the store is the app's, and a janitor for
  unreferenced files stays the app's business (ADR-0017).
