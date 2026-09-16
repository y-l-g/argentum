# Shell parity with shadcn sidebar

Date: 2026-08-28 — Status: accepted — Supersedes: none

Argentum's Shell (`render_shell` in `panel.rs` + `SIDEBAR=class!(...)` in `sidebar.rs`) flowed with the page (`flex min-h-screen`), not fixed; header `h-16` was not sticky; `sidebar_trigger` and `data-theme-toggle` buttons were inert (no JS, no cookie, no `Sheet` mobile drawer). The spec demands shadcn-grade shell: sticky/fixed, collapsible to icon rail, persisted, dark-mode toggle working.

We adopt the shadcn `sidebar.tsx` pattern verbatim but via Topcoat conventions: `SidebarProvider` sets CSS vars `--sidebar-width:16rem / --sidebar-width-icon:3rem / --sidebar-width-mobile:18rem`, gap div `w-(--sidebar-width)` with `transition-[width]`, container `fixed inset-y-0 z-10 hidden h-svh w-(--sidebar-width)` + `SidebarContent flex-1 overflow-auto`, header/footer `shrink-0` with `sticky top-0`, collapsible `data-state=expanded|collapsed` + `group-data-[collapsible=icon]` rules, cookie `sidebar_state` (604800s, `topcoat::cookie`) read server-side for SSR, keyboard `Ctrl+B`, mobile branch `if isMobile return <Sheet>` via existing `sheet` component, and `theme.js` toggling `document.documentElement.classList` (`dark`) + `localStorage` (fallback cookie) for `NextThemes` parity. JS is two minimal assets (`assets/sidebar.js`, `assets/theme.js`) included via `asset!` and `topcoat::runtime::script()`, no `topcoat_core` internals needed.

Considered: flow sidebar only (rejected: scrolls away, fails spec), `topcoat_core::ViewBuffer` hack for state (rejected: internal, not needed).

Consequences: `argentum-ui/src/components/composites/sidebar.rs` gains `SidebarProvider` + `SidebarInset`; `crates/argentum-core/src/panel.rs:98` `render_shell` injects provider, reads cookie, emits scripts. `styles.css` gains `--sidebar-*` vars (tokens). No new external gap; only public `cookie`/`asset`/`view` APIs are used.

## Amendment (2026-09-16)

Topcoat shipped its own `sidebar` component (#419), so the shell now composes the **primitive** instead of the owned composite (ADR-0007 status note). Its open state is runtime state: `render_shell` wraps the shell in a hoisting body, creates `Signal<bool>`s for the desktop panel (`open`, seeded by `sidebar_state` so the first paint matches the last choice) and the mobile sheet (`mobile_open`), and the trigger pair carries `@click` handlers; the component renders the sheet itself, so the duplicated desktop/mobile navigation trees collapse into one shared rendering. Breakpoints follow upstream (`md` instead of `lg`), the rail stays intentionally unrendered, and `assets/sidebar.js` shrank to persistence (`data-state` → cookie) and the `Ctrl+B` shortcut; `theme.js` is unchanged.
