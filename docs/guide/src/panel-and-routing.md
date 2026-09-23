# Panel and routing

How a panel is mounted, the routes a resource adds, and the panel options that shape the shell.

`Panel` owns the router, the `Db` in app context, and the shell layout. Registering a resource adds
its routes and its sidebar item.

Routes for a resource with slug `users` under prefix `admin`:

- `GET /admin/users` : list
- `GET + POST /admin/users/create` : create
- `GET /admin/users/{id}` : detail page, read-only (GH #187) — 404 when the resource declares no
  `view` schema,
  see [Detail pages](./detail-pages.md)
- `GET + POST /admin/users/{id}/edit` : edit
- `POST /admin/users/{id}/delete` : delete with confirm step
- `POST /admin/users/bulk-delete` : bulk delete
- `GET /admin/users/export` : CSV export
- `GET /admin/users/options` : relation option search for a searchable select (GH #150),
  see [Forms](./forms.md)
- `GET /admin` redirects to the first resource

Useful panel options:

```rust
Panel::new("admin")
    .brand(Brand::new("Acme"))
    .dark_mode(true)
    .login_hint("Demo: admin@example.com / password")
```

`brand` sets the header and sidebar name. `dark_mode` sets the theme a visitor sees **before they
have chosen one** — the toggle is always rendered, and a stored choice wins in both directions
(GH #184): picking light persists, and the next page stays light instead of falling back to this
default. Omit `dark_mode` and the panel starts light.

The panel owns the URL of each resource's list page and resolves a resource's sidebar entry to
`{prefix}/{slug}`; the resource owns the label and the ordering. See
[Resources](./resources.md) for the `navigation()` override.

## Public pages

A public page is an app-level `#[page]` outside the panel prefix. The auth gate installs two layers
— the panel prefix and `/_topcoat/runtime` — and a layer wraps only the routes under its path
prefix, so no route under another prefix is gated. Route discovery is link-time over the binary, so
the `discover()` call inside `Panel::build` collects these pages with no router change:

```rust
use topcoat::{
    Result,
    router::{Slot, layout, page},
    tailwind,
    view::{View, view},
};

// The layout path is a prefix: `/blog` wraps `/blog` and `/blog/{id}`, and
// nothing else. A layout at `/` would wrap `/admin` too.
#[layout("/blog")]
async fn blog_layout(slot: Slot<'_>) -> Result<impl View> {
    Ok(view! {
        <!DOCTYPE html>
        <html>
            <head>
                topcoat::dev::script()
                topcoat::runtime::script()
                <link rel="stylesheet" href=(tailwind::stylesheet!())>
            </head>
            <body>
                (slot)
            </body>
        </html>
    })
}

#[page("/blog")]
async fn blog() -> Result<impl View> {
    Ok(view! { "Posts" })
}
```

A public page renders its own document: `Panel::render_document` and `Panel::layout_shell` are the
admin shell, so the layout carries the head — `topcoat::dev::script()`,
`topcoat::runtime::script()`, `topcoat::font::link(font: …)` and
`<link rel="stylesheet" href=(tailwind::stylesheet!())>` — the same way the panel's shell does. The
runtime script, the font and the stylesheet are `Asset` URLs, and an `Asset` panics where no asset
config is registered, so a router built without `.assets(..)` — a markup test, say — needs a layout
that leaves them out: the panel's shell drops back to `topcoat::dev::script()` and its theme script
in that case, and a layout takes the same fallback by guarding on
`try_app_context::<AssetConfig>(cx).is_some()`.

The panel's resource loaders are panel-scoped (auth, tenancy, chrome), so a public page queries the
model directly: `Post::filter(Post::fields().status().eq("published".to_string()))`, with an explicit
`.include(..)` for every relation the page reads. See [Data access](./data-access.md). The framework
scopes a `Resource`'s loaders, not a page's own query, so app code that loads rows outside those
loaders calls `scoped_query` — a public page in a multi-tenant app states its tenant predicate in the
query it writes (ADR-0002).
