# Media library: a WordPress-style `medias` table in the showcase

Date: 2026-09-23 — Status: accepted — Amended: 2026-09-26

## Decision

**The library is the app's, and its rows carry no owner.** `tablo-core` keeps the seam
ADR-0017 drew — a `FileUpload` binds a `String`, the bytes go to the app's `Uploader`, and the
stored value renders as a link — and the media library is the showcase's:
`examples/showcase/src/models.rs` declares `MediaAsset`, `#[table = "medias"]`, one row per stored
file with the tenant that uploaded it, the `path` the `Uploader` returned, the client's
`filename`, a `kind`, and a timestamp. `examples/showcase/src/media.rs` is the page that fills and
renders it.

**One media source.** A post shows one library row as its cover through its own `cover_id`: an
optional single picker (`Select::relationship` against the `MediaLibrary` source) storing the
picked row's id. The post form uploads no bytes itself, and the `Media`/`Poster`/`Credit` embedded
enum is gone — the nested-embed demo is the `Seo` struct only. The public blog resolves the cover
through the picked row, and the admin detail shows the reading stats the body computes.

**The upload form is file-only.** `GET /admin/media` renders the tenant's rows and a file input;
no owner picker, no optgroups. Tenant scoping is the page's own filter on the row's `tenant_id`,
and a tenantless request is refused rather than served every tenant's rows.

**The vocabulary is two rows.** A **`MediaAsset`** is a file in the library, with bytes behind its
`path`. A post's **`cover_id`** names one such row, if any. The table keeps the issue's name
(`medias`) through `#[table = "medias"]`, because the model name is what would collide.

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
owns this store now that the post form uploads nothing directly.

**The rich upload UX is the app's, and its no-JS fallback is a reset button.** The page renders the
file input, a preview region (`data-media-preview`), and an × (`data-media-clear`) that is a
`type="reset"` control. With no script the browser resets the form: the file input empties, which is
the fallback. `examples/showcase/assets/media.js` narrows that: it empties the file input and the
preview itself and cancels the reset. The script draws the preview in the first place — an `<img>`
from an object URL for an `image/*` file, the file's name otherwise, the URL revoked when the
preview is replaced or cleared — and a form with no file input in reach keeps the browser's reset
rather than swallowing the click.

The script is the **app's asset**, declared as `MEDIA_JS` and linked `defer`red by the page rather
than added to the shell's set. ADR-0014 owns the scripts `tablo-ui` ships — the document emits
them, and no component emits its own — and this is the app's own script on the app's own page: a
tenth shell asset would load media-widget code into every admin document of every app (ADR-0014's
all-load policy) for a widget one page renders, and the framework offers no seam for an app-supplied
shell script at all. The app owns its script the way it owns its `styles.css` (ADR-0006).

**A thumbnail is decided by content type, not by a suffix.** A row's `kind` is `"image"` when the
uploaded part's `Content-Type` is `image/*`, and `"file"` otherwise. The framework deleted its
extension check and its preview (GH #242) because a filename is not a content type; classifying is
the app's, and what it classifies is what the browser said the bytes are. An image row renders
`<img>`, anything else a link, through one view the media page and the public blog's cover share.
The library generates no derivative: the thumbnail is the stored image sized down in the page, so
nothing here reads or rewrites the bytes after the store returns.

## Consequences

- `examples/showcase/tests/media_check.rs` covers the upload path (the store's URL fetched back, a
  basename that cannot climb out of the served directory, filenames a URL would otherwise break —
  `#`, a space, `%22`, `%`, a trailing space), the file-only form, the thumbnail/link split, the
  reset control, the picked cover on the blog page, and the refusals (a missing CSRF token, a
  tenantless request). `examples/showcase/assets/media.test.js` covers the script, and CI's `assets`
  job runs it.
- The library lists one tenant's rows — the uploader's. The row's own `tenant_id` is what the page
  filters on.
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
