# Shell parity with shadcn sidebar

Date: 2026-08-28 — Status: accepted — Amended: 2026-09-16

## Decision

The Shell is sticky/fixed and collapsible, following the shadcn `sidebar.tsx` pattern through Topcoat
conventions and the upstream `sidebar` primitive (topcoat#419, ADR-0007). It uses the shadcn CSS
variables (`--sidebar-width:16rem`, `--sidebar-width-icon:3rem`, `--sidebar-width-mobile:18rem`), a
fixed container with `SidebarContent` scrolling inside it, a `shrink-0` sticky header/footer, and
`data-state=expanded|collapsed`; below `md` the component renders its own sheet drawer. The icon rail
stays intentionally unrendered.

Open state is runtime state. `Panel::render_shell` wraps the shell in a hoisting body and creates
`Signal<bool>`s for the desktop panel (`open`, seeded from the `sidebar_state` cookie so the first
paint matches the last choice) and the mobile sheet (`mobile_open`); the trigger pair carries
`@click` handlers, and the component renders the desktop and mobile navigation as one tree.
`assets/sidebar.js` persists `data-state` back to the cookie (604800s) and binds `Ctrl+B`;
`theme.js` toggles `document.documentElement.classList` (`dark`) with `localStorage` (fallback
cookie) for `NextThemes` parity, and the blocking `theme_init_script` reconciles the class before
first paint. Both scripts are emitted with `asset!` + `topcoat::runtime::script()`, and the full
asset list and hook contract are ADR-0014.
