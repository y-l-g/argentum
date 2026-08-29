fn main() {
    // Stage the Feather icon set for `iconify_icon!` in primitives (ADR-0007
    // verbatim sync). Staging downloads the set from the Iconify CDN into
    // OUT_DIR and never needed a local topcoat checkout — the sniffing of
    // `../../../topcoat` dates from the sibling-clone era and silently
    // disabled icons whenever the clone was missing.
    topcoat::icon::iconify::BuildConfig::new()
        .icon_set("feather")
        .stage()
        .unwrap();
    // Tailwind is per-app (`argentum_ui::tailwind_build` in app's build.rs), nothing to do here.
}
