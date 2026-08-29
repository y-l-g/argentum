# Primitives vs composites in argentum-ui

`argentum-ui/src/components/` mixed re-exported Topcoat UI primitives (button, card, badge...) with owned composites (sidebar) in one flat folder, so a `topcoat-ui-registry` sync would conflict with Argentum edits. We split into `primitives/` — verbatim mirror of `topcoat-ui-registry/src/components/` synced via `cargo xtask sync-topcoat-ui` (header `// SYNC: topcoat-ui-registry@<commit> — do not hand-edit`) — and `composites/` — hand-written Argentum components (Sidebar, Page, CodeBlock) that compose primitives and Tokens. One crate is kept (`argentum-ui` re-exports both) to avoid crate proliferation; a two-crate split (`argentum-ui-primitives` + `argentum-ui`) was rejected.

Considered: (A) two crates, (B) `topcoat ui add` copy-source vendoring per app (rejected: reintroduces breakage on upgrade, ADR-0006).

Consequences: `primitives/` is overwritten by the sync task; `composites/` is never overwritten. `lib.rs` re-exports both but docs distinguish the origin. Future Topcoat components (e.g. upstream `sidebar` once PR'd) land in `primitives/` and `composites/sidebar.rs` becomes a thin wrapper or is retired.

**Status 2026-08-29:** the legacy flat `components/*.rs` tree is deleted and `lib.rs` re-exports point at `components::primitives::*` (previously they re-exported the stale flat copies, which had drifted from the registry — e.g. a hand-edited `card`). `primitives/` is now the single source for synced components; `composites/` holds sidebar, page, code_block and theme.

**Status 2026-08-29 (git deps):** Topcoat/Toasty are git dependencies (`branch = "main"`, pinned by `Cargo.lock`); the sibling-clone requirement is gone. `sync-topcoat-ui` resolves `topcoat-ui-registry` through `cargo metadata` (same mechanism as `topcoat ui`) and reads sources via the registry API, so synced content always matches what Cargo compiles. The SYNC header identifies the registry by crate version (`topcoat-ui-registry@0.6.2`) instead of a sibling-clone commit.
