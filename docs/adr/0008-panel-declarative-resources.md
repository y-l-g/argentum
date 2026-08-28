# Panel declares Resources, Shell is implicit

`examples/showcase/src/app.rs:57` `admin_layout` was 35 lines of manual `<!DOCTYPE><html><head> if has_assets ...>` plus `vec![Panel::new("admin").navigation_item::<UserResource>(), NavigationItem{Showcase}]` and `Panel::render_shell(..., &current, inner, None)`. This mixes declarative `Resource::table`/`Resource::form` with imperative shell HTML, forcing every app to copy-paste the layout and the `try_app_context::<AssetConfig>` fallback for tests. Filament provides `Panel::resources([UserResource])` out of the box.

We make `Panel` declarative as the primary seam: `Panel::new("admin").resource::<UserResource>().page(showcase::index).build()` discovers `#[page]`/`#[layout]` via `Router::builder().discover()` and provides a default `argentum_shell` `#[layout("/admin")]` that owns `tailwind::stylesheet!()`, `fontsource_font!(GEIST)`, `topcoat::runtime::script()` and the `has_assets` fallback internally. `Panel::render_shell` stays as the low-level primitive for custom layouts (e.g. showcase's extra "Showcase" nav item); apps only define a custom `#[layout]` when they need to override the default. `NavigationItem` moves toward typed `Href` via `Href::is_current` (Topcoat `d273cb15`) instead of string prefix.

Considered: (A) keep only manual layout (rejected: boilerplate, diverges from Filament DX), (B) proc-macro `#[panel]` (deferred).

Consequences: empty `cargo new` app with one `Resource` renders a Filament-grade shell with zero HTML. `examples/showcase` keeps its custom layout as the reference for escape-hatch usage. `Panel` remains the single owner of Router/Db/Shell per `CONTEXT.md`.
