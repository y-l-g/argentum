//! Guards that the primitives vendored into `argentum-ui` stay verbatim with
//! the `topcoat-ui-registry` sources this workspace compiles against — the
//! same contract topcoat's own `examples/ui/tests/registry_sync.rs` enforces
//! for its example app. Because the sync is byte-for-byte (no string patches,
//! no injected content inside the source), the SYNC header's sha256 is
//! meaningful and drift cannot hide: a hand edit, a stale file, or a missing
//! component fails here until `cargo xtask sync-topcoat-ui` restores it.

#[test]
fn primitives_match_registry_verbatim() {
    xtask::verify_sync().expect("vendored primitives match the registry");
}
