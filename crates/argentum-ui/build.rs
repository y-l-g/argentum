fn main() {
    // Stage Feather icon set for `iconify_icon!` in primitives (ADR-0007 verbatim sync).
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // `crates/argentum-ui` -> `..` = `crates`, `..` = `argentum`, `..` = `filament-topcoat`
    let topcoat_cargo = manifest.join("../../../topcoat/crates/topcoat/Cargo.toml");
    if topcoat_cargo.exists() {
        topcoat::icon::iconify::BuildConfig::new()
            .icon_set("feather")
            .stage()
            .unwrap();
    }
    // Tailwind is per-app (`argentum_ui::tailwind_build` in app's build.rs), nothing to do here.
}
