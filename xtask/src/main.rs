fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    match cmd.as_str() {
        "sync-topcoat-ui" | "sync" => {
            let dry_run = args.any(|a| a == "--dry-run");
            xtask::sync_topcoat_ui(dry_run)?;
        }
        "verify-topcoat-ui" | "verify" => {
            xtask::verify_sync()?;
        }
        "--help" | "-h" | "help" => {
            print_help();
        }
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            std::process::exit(1);
        }
    }
    Ok(())
}

fn print_help() {
    println!(
        r#"xtask — repo tasks (ADR-0007)

USAGE:
    cargo xtask sync-topcoat-ui [--dry-run]
    cargo xtask verify-topcoat-ui

COMMANDS:
    sync-topcoat-ui    Copy primitives from the `topcoat-ui-registry` crate
                       Cargo resolved for this workspace into
                       crates/argentum-ui/src/components/primitives/*.rs —
                       verbatim, under a SYNC header recording the registry
                       version and the source's sha256 content hash. Never
                       touches composites/. No sibling clone required — the
                       registry comes from the same git source Cargo
                       compiles against.
    verify-topcoat-ui  Guard: fail when any vendored primitive (or mod.rs)
                       has drifted from the registry. The xtask test suite
                       runs this on every `cargo test`.

OPTIONS:
    --dry-run          Print what would be copied without writing
    --help, -h         Show this help
"#
    );
}
