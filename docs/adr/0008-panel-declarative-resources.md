# Panel declares Resources, Shell is implicit

Date: 2026-08-28 — Status: accepted — Amended: 2026-09-19, 2026-09-22

## Decision

`Panel` is the declarative seam:
`Panel::new("admin").resource::<UserResource>().build().expect("panel builds")` discovers
`#[page]`/`#[layout]` via `Router::builder().discover()`, registers each resource's routes (the list
page at `{prefix}/{slug}`), and redirects the panel root to the first resource. An app's layout
delegates to `Panel::layout_shell`, which owns the complete document, so applications carry no
document HTML or `AssetConfig` fallback; `shell_assets(tailwind::stylesheet!(), font)` supplies the
call-site assets the app's Tailwind scan needs, and `Panel::render_shell` stays the low-level
primitive for custom layouts.

`Panel::nav_item` is the one panel-aware navigation seam, and it consumes the resource's
`Resource::navigation()`: the resource owns label, order and grouping, the Panel owns the URL. The
default entry names no URL at all — `NavTarget::Derived` — and the Panel that owns the item resolves
that target from its own mount prefix plus the resource's slug (`{prefix}/{slug}`). An explicit
target (`NavTarget::Url`, which `NavigationItem::at` builds) is kept verbatim, so resolution can
never rewrite a link its author wrote, not even one shaped like `/admin/{slug}`.
`NavigationItem::for_resource` is the derived constructor and `at` the explicit one, so no public
constructor can emit a wrong mount; `is_current_path` matches exact paths or slash-boundary
prefixes, and an override that wants a different order sets the `order` field.

## Consequences

An app with one `Resource` gets a Filament-grade shell by delegating its layout to
`Panel::layout_shell`, with zero document or sidebar HTML in application code. `examples/showcase` is
the reference for the asset-loading contract and for typed navigation. `Panel` remains the single
owner of Router/Db/Shell per `CONTEXT.md`.
