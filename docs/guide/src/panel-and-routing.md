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
