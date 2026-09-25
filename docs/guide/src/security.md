# Security

The defaults Argentum ships with, and the deployment assumptions they depend on.

- All POSTs verify a double-submit `csrf_token` before any DB work. `confirm=1` is a UX step, not a
  boundary.
- A `FileUpload` value comes only from a file part (the uploader's answer, or the sanitized basename
  with no uploader), the record's stored value on an untouched edit, or empty on `clear_<field>`. A
  stored value renders as a link only when it is rooted (`/…`, not `//host`) or an absolute
  `http(s)://…` URL; anything else renders as text (GH #277).
- Passwords use Argon2id. Unknown emails take the same code path, and login failures share one
  generic message.
- Deletes and bulk deletes re-fetch through `query()` and re-check policy inside the handler
  transaction.
- Table free-text is an escaped substring `LIKE` across the searchable columns (`like_with_escape` +
  `escape_like_pattern`), never a raw pattern. Do not interpolate raw input into SQL.
- Session, CSRF, and notification cookies use hardened `__Host-` + `Secure` settings. Localhost is
  exempt; non-localhost deploys need HTTPS or browsers drop them and mutations 403.
- Every response the panel's layer chain produces, including the router's own 404 and 405, carries
  `Content-Security-Policy: frame-ancestors 'self'`, so an admin page cannot be clickjacked from
  another origin. `Panel::frame_ancestors(..)` widens it for a deployment that frames the panel,
  `Panel::without_frame_ancestors()` sends none for a proxy that owns the whole policy, and an
  app's own `Content-Security-Policy` on a handler response always wins — the layer only fills the
  gap. Three router responses are built outside the layers and so carry no directive: the 403 for a
  cross-site request, the 400 for a malformed `x-topcoat-identity` header, and the bare 500 for a
  panic.
- Redirects: `Err(redirect(..))` (307) for GETs, `Err(see_other(..))` (303 PRG) after mutations.
  Mid-stream they degrade to `window.location.replace`; streamed regions own their failure
  rendering. Wrap `Slot` in `error_boundary` for branded error pages.
- A served directory (`Panel::serve_dir`) shares the panel's origin, so every file response the
  directory route serves carries `X-Content-Type-Options: nosniff`, a fixed sandboxing
  `Content-Security-Policy`, and `Content-Disposition: attachment` unless the file is a common raster
  image, audio/video or `text/plain`. An app that serves active documents mounts them on its own
  origin; the policy is not configurable. The directory route's own failures keep Topcoat's
  `text/plain` 404 page and an `Allow`-only empty 405, so they carry none of the served-file policy
  and no user content; the panel's `frame-ancestors` layer still covers both.
