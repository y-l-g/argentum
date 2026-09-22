# Primitives vs composites in argentum-ui

Date: 2026-08-28 — Status: accepted — Amended: 2026-08-29, 2026-09-16, 2026-09-19

## Decision

`argentum-ui/src/components/` splits in two. `primitives/` is a verbatim mirror of
`topcoat-ui-registry/src/components/`, synced by `cargo xtask sync-topcoat-ui`; every synced file
carries the header `// SYNC: topcoat-ui-registry@<version> sha256:<content-hash> — do not hand-edit`,
and drift is enforced by `xtask/tests/registry_sync.rs`. `composites/` holds the hand-written
Argentum components — page, error_state, theme, toast — which compose primitives and Tokens and are
never overwritten by the sync. One crate, `argentum-ui`, re-exports both and its docs distinguish the
origin.

`sync-topcoat-ui` resolves `topcoat-ui-registry` through `cargo metadata` (the same mechanism as
`topcoat ui`) and reads sources through the registry API, so synced content always matches what Cargo
compiles. Topcoat and Toasty are git dependencies (`branch = "main"`, pinned by `Cargo.lock`), so no
sibling clone is required. The upstream `sidebar` is synced into `primitives/sidebar.rs` and
`lib.rs` re-exports the primitive; the shell binds its runtime signals directly (ADR-0009).

A two-crate split (`argentum-ui-primitives` + `argentum-ui`) is rejected as crate proliferation, and
`topcoat ui add` copy-source vendoring per app is rejected for the upgrade breakage it reintroduces
(ADR-0006). New upstream components are added to `primitives/`; owned components stay in
`composites/`.
