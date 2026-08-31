# Panel declares Resources, Shell is implicit

`examples/showcase/src/app.rs:66` now has a one-line `admin_layout` delegating to `Panel::layout_shell`; resource routes and navigation are declared on the Panel rather than hand-written in the application router. This keeps the shell escape hatch explicit without forcing every app to copy-paste document HTML or an `AssetConfig` fallback. Filament provides `Panel::resources([UserResource])` out of the box.

We make `Panel` declarative as the primary seam: `Panel::new("admin").resource::<UserResource>().navigation(NavigationItem::from_href(...)).build()` discovers `#[page]`/`#[layout]` via `Router::builder().discover()`, registers the list route at `/admin/users`, and redirects `/admin` to the first resource. `Panel::layout_shell` owns the complete document, while `shell_assets(tailwind::stylesheet!(), font)` supplies the call-site assets required by the app's Tailwind scan. `Panel::render_shell` stays as the low-level primitive for custom layouts. `NavigationItem` uses typed `Href::is_current` where a custom page supplies an href and slash-boundary matching for resource paths.

Considered: (A) keep only manual layout (rejected: boilerplate, diverges from Filament DX), (B) proc-macro `#[panel]` (deferred).

Consequences: an app with one `Resource` gets a Filament-grade shell by delegating its layout to `Panel::layout_shell`, with zero document or sidebar HTML in application code. `examples/showcase` is the reference for the asset-loading contract and custom typed navigation. `Panel` remains the single owner of Router/Db/Shell per `CONTEXT.md`.
