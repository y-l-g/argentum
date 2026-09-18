//! Guards the shell-JS hook contract (GH #152, ADR-0014): every hand-written
//! asset under `crates/argentum-ui/assets/` still exists and stays wired to
//! its `argentum-ui` constant, and every hook in the checked-in
//! [`xtask::ASSET_HOOKS`] list still appears in both its JS asset and a Rust
//! render site. A missing/renamed asset or a rename on either side of a
//! string-selector coupling fails here, because `asset!` does not stat its
//! source at compile time and nothing else checks the JS consumer side.

#[test]
fn shell_assets_match_hook_contract() {
    xtask::verify_asset_hooks().expect("shell JS assets match the hook contract");
}
