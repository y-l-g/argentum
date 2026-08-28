# Primitives vs composites in argentum-ui

`argentum-ui/src/components/` mixed re-exported Topcoat UI primitives (button, card, badge...) with owned composites (sidebar) in one flat folder, so a `topcoat-ui-registry` sync would conflict with Argentum edits. We split into `primitives/` — verbatim mirror of `topcoat-ui-registry/src/components/` synced via `cargo xtask sync-topcoat-ui` (header `// SYNC: topcoat-ui-registry@<commit> — do not hand-edit`) — and `composites/` — hand-written Argentum components (Sidebar, Page, CodeBlock) that compose primitives and Tokens. One crate is kept (`argentum-ui` re-exports both) to avoid crate proliferation; a two-crate split (`argentum-ui-primitives` + `argentum-ui`) was rejected.

Considered: (A) two crates, (B) `topcoat ui add` copy-source vendoring per app (rejected: reintroduces breakage on upgrade, ADR-0006).

Consequences: `primitives/` is overwritten by the sync task; `composites/` is never overwritten. `lib.rs` re-exports both but docs distinguish the origin. Future Topcoat components (e.g. upstream `sidebar` once PR'd) land in `primitives/` and `composites/sidebar.rs` becomes a thin wrapper or is retired.
