# Beautiful primitives in argentum-ui, with the Tailwind seam per app

Date: 2026-08-28 — Status: accepted — Amended: 2026-09-10, 2026-09-16, 2026-09-22

## Decision

Argentum must be beautiful by default — a new project that defines a Panel and a Resource gets a
Filament-grade dashboard with no extra setup — yet Topcoat UI is copy-source (`topcoat ui init` +
`topcoat ui add` drops owned files into the app). The beautiful primitives live in the
`argentum-ui` crate, which depends on `topcoat-ui-registry` as a library and re-exports styled
`#[component]`s (table, card, input, label, button, pagination, skeleton, dialog, separator, sheet,
sidebar and the rest). `argentum-core`'s `Table`, `Schema` and `Panel` shell render those components
directly; apps never run `topcoat ui add` and never own component source, so an upgrade cannot be
broken by local edits.

The Tailwind seam stays per-app and explicit: `styles.css` (importing `tailwindcss` + neutral tokens
+ `@source` for both the app's `src/**/*.rs` and `argentum-ui/src/**/*.rs`) and a 3-line `build.rs`
(`argentum_ui::tailwind_build()` / `topcoat::tailwind::BuildConfig`) plus `tailwind::stylesheet!()`
and the Geist font in the layout. That is the mechanism Topcoat documents, it keeps tree-shaking per
app, and it makes token editing the single customization seam: change `--primary`, `--background`,
etc. in `:root`/`.dark`. There is no Rust `Panel::theme` builder and no per-cell `attrs`; the narrow
class seam is `Section::class`, merged via `class!` against the token classes rather than replacing
them. `argentum-ui` is the one `@source` an app's stylesheet must add, and that stays an ADR-visible
contract until Topcoat documents dependency scanning natively.

The Sidebar is an upstream `topcoat-ui-registry` component (topcoat#419), vendored into
`primitives/` by `cargo xtask sync-topcoat-ui`. Its `sidebar_menu_button` takes `active` (not
`is_active`) plus `href`/`tooltip` props, the trigger pair and rail carry `@click` handlers, and
`open`/`mobile_open` are runtime expressions (`Signal<bool>`) with the mobile sheet owned by the
component. `Panel::render_shell` binds the signals, seeds `open` from the `sidebar_state` cookie, and
`assets/sidebar.js` keeps the cookie and `Ctrl+B`.

## Consequences

`examples/showcase` is the reference for the stylesheet and build setup, not a template to copy;
empty projects follow the docs (one `styles.css`, one `build.rs`) until a scaffold automates them.
`Panel::layout_shell` owns the document links, the app passes its generated stylesheet and font
handles through `Panel::shell_assets`, and `Panel::assets` owns the loaded bundle. The primitives
sync is version + sha256-guarded by `xtask/tests/it.rs`. Publishing an
`argentum-ui-registry` for `topcoat ui add --registry argentum` is rejected: it reintroduces the
copy-source steps and the upgrade breakage. Embedding a prebuilt stylesheet in `argentum-ui` and
injecting it automatically stays deferred: it hides Tailwind's build, loses per-app tree-shaking,
and may return as an opt-in feature for demos.
