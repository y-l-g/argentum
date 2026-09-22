# Shell JS assets: ownership, all-load policy, and the hook contract

Date: 2026-09-18 — Status: accepted — Supersedes: none

## Context

`crates/argentum-ui/assets/` holds eight hand-written JS assets (`sidebar.js`,
`theme.js`, `dialog.js`, `bulk.js`, `filters.js`, `live-search.js`,
`selects.js`, `notifications.js`; ~14 KB unminified, ~4.7 KB gzipped, no
build/minify step). They are declared as `Asset` constants in
`crates/argentum-ui/src/lib.rs` (`SIDEBAR_JS` … `NOTIFICATION_JS`) and emitted
by `Panel::render_document` in `argentum-core` on every document with
`ShellAssets` — including the login page, where all but `theme.js`'s backstop
apply are no-ops. Every asset is coupled to markup by string selectors
rendered anywhere from `argentum-ui` composites to `argentum-core` pages, with
nothing compiler-checked: a rename on the JS side is always silent, and
`asset!` does not stat its source at compile time, so even a deleted `.js`
passes `cargo test`. Components do not reference their assets, `dialog`/`sheet`
are vendored primitives under the ADR-0007 sync guard (so they cannot carry
rustdoc), and ADR-0009 still describes a two-asset world.

## Decision

**Ownership.** The assets live in `argentum-ui` (declared constants, shipped
sources) but the scripts are owned by the document: only
`Panel::render_document` emits `<script>` tags, `defer`red (GH #152 — parsing
never waits for them; every asset either registers document-level listeners at
execution or binds in a `DOMContentLoaded` handler, which deferred scripts
still run before). The blocking `theme_init_script` stays inline so the `dark`
class lands pre-paint. Per-component `<script>` tags stay out: duplicate
execution stacks document listeners (sidebar toggle becomes a no-op, theme
toggle dies, the filters submit path fires twice), and the runtime does not
manage script lifecycles in swapped content.

**All-load policy.** Every document with `ShellAssets` loads all eight
scripts. `render_document` receives an opaque `BoxView` and `layout_shell` a
lazy `Slot`, so nothing at document level can observe what was rendered;
scoping emission to page content needs a new declaration API, and it would not
shrink the bundle — all eight handles stay referenced, so the bundler still
ships all eight. Revisit only if asset weight grows or the tag count becomes a
real problem. The "hook ⇒ script" guarantee therefore holds only for documents
rendered through `render_document` with `ShellAssets` configured: a `Panel`
built without `.shell_assets(..)` renders sidebar/toaster hooks with no
scripts, as do apps using `argentum-ui` components directly.

**Hook contract.** Each asset consumes an explicit hook list, guarded by
`xtask/tests/asset_hooks.rs` (via `xtask::verify_asset_hooks`, mirroring
`registry_sync.rs`): the test fails when an asset file is missing/renamed or
when a listed hook no longer appears in both its JS asset and the Rust
sources. The list is attribute hooks only — structural selectors (`.relative`,
`pre code`, `select option`, `dialog[open]`) and the inverse direction (a
rendered hook with no consumer) are out of scope, per the #136 anti-brittle
bar: presence of hook strings, never classes or pixel markup. Track new hooks
in the list as they land (the live-filter `data-filters-live` direction from
#151 is already in it).

| Asset | Hooks (Rust render site → JS consumer) | Without the script |
|---|---|---|
| `sidebar.js` | `data-sidebar` / `data-state` (sidebar primitive), `sidebar_state` cookie (shell) | State no longer persists; `Ctrl+B` dies |
| `theme.js` | `data-theme-toggle` (shell) | Toggle inert; init script still paints the stored theme |
| `dialog.js` | `data-dialog-close`, `data-dialog-open-param` (delete dialog) | Cancel still navigates, Delete still POSTs; no Escape/backdrop dismissal |
| `bulk.js` | `data-bulk-form` / `data-table-root` / `data-bulk-submit` / `data-row-select` / `data-bulk-select-all` / `ids` transport (table) | Bulk delete unusable (submit ships disabled) |
| `filters.js` | `data-filter-name` / `data-filters-form` / `data-filters-transport` / `data-filters-live` (filter bar) | Typed controls inert; `<noscript>` free-text + Apply keeps working |
| `live-search.js` | `data-live-search` / `data-live-search-input` / `data-live-search-transport` / `data-debounce-ms` (live table toolbar) | Typing no longer debounces into a reload; the `<noscript>` GET form is the search path |
| `selects.js` | `data-select-filterable` / `data-options-filter` (searchable `Select`) | Filter input inert; plain select keeps working |
| `notifications.js` | `data-sonner-toast` / `data-close-button` / `data-mounted` (toaster) | Toasts stay visible via `<noscript>` until next navigation |

**No-build stance.** No `package.json`, no lint/format config, no Node step in
CI, no minification: the assets are small enough that a toolchain would cost
more than it saves. Revisit with the all-load policy if they grow.

**No-JS posture.** `sidebar.js` (mobile nav persistence) and `bulk.js` (bulk
delete) are load-bearing for their features; `theme.js` is needed for the
toggle; the other five are progressive enhancements with fallbacks, as the
table above records. Delegation is deliberate throughout: Topcoat morphs
swapped content with no script-lifecycle handling, so document-level listeners
(plus `notifications.js`'s `MutationObserver` for mounted-state arming) keep
behavior alive after post-load shard swaps.

**Where the requirement is documented.** Composites carry it in rustdoc
(`toaster`); `dialog`/`sheet` are vendored primitives, so the
note lives at the core render sites (the delete dialog, the `render_shell`
mobile sheet) and on searchable `Select` in `schema.rs` (GH #152).

## Consequences

`Panel::render_document` keeps emitting all eight tags, `defer`red; new hooks
extend `ASSET_HOOKS` with both sides in the same commit; ADR-0009's two stale
lines (two-asset claim, `render_shell` emitting scripts) are corrected and
`README.md`'s brand/dark-mode seam is restated. `cargo xtask` still never
touches `assets/` (ADR-0007 covers primitives only).

**Status 2026-09-19 (GH #173):** `code_block.js` and its `data-copy-button` hook left the registry with the `code_block` composite — zero callers, and a per-page script for a snippet renderer nothing rendered. Re-adding a snippet view means re-adding both sides together, as the contract requires.

**Status 2026-09-22 (GH #213):** `live-search.js` (GH #172) shipped with no `ASSET_FILES`/`ASSET_HOOKS` entry, so the guard covered seven assets while the shell emitted eight: a deleted, emptied or unwired `live-search.js` — or a rename on either side of any of its four hooks — passed `cargo test -p xtask` unnoticed. The asset and all four hooks are covered now, and the table above lists exactly what `Panel::render_document` emits.
