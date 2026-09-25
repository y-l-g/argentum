# Primitives vs composites in tablo-ui

Date: 2026-08-28 — Status: accepted — Amended: 2026-08-29, 2026-09-16, 2026-09-19, 2026-09-25

## Decision

`tablo-ui/src/components/` splits in two. `primitives/` is a verbatim mirror of
`topcoat-ui-registry/src/components/`, synced by `cargo xtask sync-topcoat-ui`; every synced file
carries the header `// SYNC: topcoat-ui-registry@<version> sha256:<content-hash> — do not hand-edit`,
and drift is enforced by `xtask/tests/it.rs`. `composites/` holds the hand-written Tablo
components — page, error_state, theme, toast — which compose primitives and Tokens and are never
overwritten by the sync. One crate, `tablo-ui`, re-exports both and its docs distinguish the
origin.

The vendored set is explicit and app-owned: `xtask::VENDORED_PRIMITIVES` lists the registry
components Tablo vendors, and `sync-topcoat-ui`, the generated `mod.rs`, and `verify-topcoat-ui`
all use exactly that set. It is the transitive closure of the components `lib.rs` re-exports: the
sync and the guards check every vendored component's same-registry `dependencies` against the set and
fail by name when one is missing, so the list cannot drift from the registry's dependency graph, and
a registry component no Tablo code calls is not vendored. Adding a component is a one-line change
to the list plus a `sync-topcoat-ui` run; the orphan guard flags any file in `primitives/` outside
the set.

`sync-topcoat-ui` resolves `topcoat-ui-registry` through `cargo metadata` (the same mechanism as
`topcoat ui`) and reads sources through the registry API, so synced content always matches what Cargo
compiles. Topcoat and Toasty are git dependencies (`branch = "main"`, pinned by `Cargo.lock`), so no
sibling clone is required. The upstream `sidebar` is synced into `primitives/sidebar.rs` and
`lib.rs` re-exports the primitive; the shell binds its runtime signals directly (ADR-0009).

A two-crate split (`tablo-ui-primitives` + `tablo-ui`) is rejected as crate proliferation, and
`topcoat ui add` copy-source vendoring per app is rejected for the upgrade breakage it reintroduces
(ADR-0006). A component is vendored by adding its registry name to `VENDORED_PRIMITIVES`; owned
components stay in `composites/`.
