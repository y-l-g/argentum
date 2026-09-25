//! xtask's repo guards, in one test target so a single link covers all of them.

/// Guards the shell-JS hook contract (GH #152, ADR-0014): every hand-written
/// asset under `crates/argentum-ui/assets/` still exists and stays wired to
/// its `argentum-ui` constant, and every hook in the checked-in
/// [`xtask::ASSET_HOOKS`] list still appears in both its JS asset and a Rust
/// render site. A missing/renamed asset or a rename on either side of a
/// string-selector coupling fails here, because `asset!` does not stat its
/// source at compile time and nothing else checks the JS consumer side.
#[test]
fn shell_assets_match_hook_contract() {
    xtask::verify_asset_hooks().expect("shell JS assets match the hook contract");
}

/// Guards that the primitives vendored into `argentum-ui` stay verbatim with
/// the `topcoat-ui-registry` sources this workspace compiles against — the
/// same contract topcoat's own `examples/ui/tests/registry_sync.rs` enforces
/// for its example app. Because the sync is byte-for-byte (no string patches,
/// no injected content inside the source), the SYNC header's sha256 is
/// meaningful and drift cannot hide: a hand edit, a stale file, or a missing
/// component fails here until `cargo xtask sync-topcoat-ui` restores it.
#[test]
fn primitives_match_registry_verbatim() {
    xtask::verify_sync().expect("vendored primitives match the registry");
}

/// Guards that [`xtask::VENDORED_PRIMITIVES`] is the transitive closure the
/// `lib.rs` re-exports require: every same-registry dependency a vendored
/// component declares is itself in the set, so the list cannot drift from the
/// registry's own dependency graph without failing here by name.
#[test]
fn vendored_primitives_are_closed_under_registry_dependencies() {
    xtask::verify_vendored_closure().expect("the vendored set is closed under its dependencies");
}
