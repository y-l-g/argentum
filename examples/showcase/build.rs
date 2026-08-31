fn main() {
    // Per-app Tailwind contract: one styles.css + this 3-line build.rs.
    // See ADR-0006 and examples/showcase/styles.css.
    println!("cargo:rerun-if-changed=styles.css");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/**/*.rs");
    println!("cargo:rerun-if-changed=../../crates/argentum-core/src/**/*.rs");
    println!("cargo:rerun-if-changed=../../crates/argentum-ui/src/**/*.rs");
    // Try to build Tailwind; on failure (e.g. offline) create empty fallback so `cargo test` stays green.
    match argentum_ui::tailwind_build() {
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "warning: tailwind_build failed: {e:?} - creating empty stylesheet for offline build"
            );
            if let Ok(out_dir) = std::env::var("OUT_DIR") {
                let out = std::path::Path::new(&out_dir).join("tailwind.css");
                let _ = std::fs::write(out, "/* tailwind build failed - offline */\n");
            }
        }
    }
}
