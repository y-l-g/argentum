# Security

The defaults Argentum ships with, and the deployment assumptions they depend on.

- All POSTs verify a double-submit `csrf_token` before any DB work. `confirm=1` is a UX step, not a
  boundary.
- Passwords use Argon2id. Unknown emails take the same code path, and login failures share one
  generic message.
- Deletes and bulk deletes re-fetch through `query()` and re-check policy inside the handler
  transaction.
- Table free-text is an escaped substring `LIKE` across the searchable columns (`like_with_escape` +
  `escape_like_pattern`), never a raw pattern. Do not interpolate raw input into SQL.
- Session, CSRF, and notification cookies use hardened `__Host-` + `Secure` settings. Localhost is
  exempt; non-localhost deploys need HTTPS or browsers drop them and mutations 403.
- Responses carry `Content-Security-Policy: frame-ancestors 'self'`, so an admin page cannot be
  clickjacked from another origin. `Panel::frame_ancestors(..)` widens it for a deployment that
  frames the panel, `Panel::without_frame_ancestors()` sends none for a proxy that owns the whole
  policy, and an app's own `Content-Security-Policy` always wins — the layer only fills the gap.
- Redirects: `Err(redirect(..))` (307) for GETs, `Err(see_other(..))` (303 PRG) after mutations.
  Mid-stream they degrade to `window.location.replace`; streamed regions own their failure
  rendering. Wrap `Slot` in `error_boundary` for branded error pages.
- A served directory (`Panel::serve_dir`) shares the panel's origin, so every file response the
  directory route serves carries `X-Content-Type-Options: nosniff`, a fixed sandboxing
  `Content-Security-Policy`, and `Content-Disposition: attachment` unless the file is a common raster
  image, audio/video or `text/plain`. An app that serves active documents mounts them on its own
  origin; the policy is not configurable. A 404 keeps Topcoat's `text/plain` error page; a 405
  carries only `Allow` and an empty body. Neither carries user content.
