use std::path::{Path, PathBuf};

use topcoat_ui::Registry;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    match cmd.as_str() {
        "sync-topcoat-ui" | "sync" => {
            let dry_run = args.any(|a| a == "--dry-run");
            sync_topcoat_ui(dry_run)?;
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

COMMANDS:
    sync-topcoat-ui    Copy primitives from the `topcoat-ui-registry` crate
                       Cargo resolved for this workspace into
                       crates/argentum-ui/src/components/primitives/*.rs
                       with a SYNC header. Never touches composites/.
                       No sibling clone required — the registry comes from
                       the same git source Cargo compiles against.

OPTIONS:
    --dry-run          Print what would be copied without writing
    --help, -h         Show this help
"#
    );
}

/// The registry Cargo resolved for this workspace, plus its crate version.
///
/// Located through `cargo metadata` — the same mechanism `topcoat ui` itself
/// uses (topcoat-ui/src/manage/workspace.rs) — so the synced sources always
/// come from the exact `topcoat-ui-registry` the workspace compiles against,
/// pinned by `Cargo.lock`. The registry directory is read from the data
/// crate's `[package.metadata.topcoat-ui] registry` declaration.
fn locate_registry() -> anyhow::Result<(Registry, String)> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|error| anyhow::anyhow!("failed to run cargo metadata: {error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| anyhow::anyhow!("could not parse cargo metadata: {error}"))?;

    let package = metadata["packages"]
        .as_array()
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package["name"] == "topcoat-ui-registry")
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "`topcoat-ui-registry` is not in the dependency graph — it must be a \
                 dependency of xtask (see xtask/Cargo.toml)"
            )
        })?;

    let version = package["version"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("topcoat-ui-registry has no version in cargo metadata"))?
        .to_string();
    let manifest_path = package["manifest_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("topcoat-ui-registry has no manifest_path"))?;
    let relative = package["metadata"]["topcoat-ui"]["registry"]
        .as_str()
        .unwrap_or(".");
    let dir = Path::new(manifest_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(relative);

    let registry = Registry::load(dir)?;
    Ok((registry, version))
}

fn sync_topcoat_ui(dry_run: bool) -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // xtask is at <repo>/xtask, so repo root is parent of manifest_dir
    let repo_root = manifest_dir.parent().unwrap_or(Path::new("."));
    let dst_dir = repo_root.join("crates/argentum-ui/src/components/primitives");
    std::fs::create_dir_all(&dst_dir)?;

    let (registry, version) = locate_registry()?;
    let header = format!(
        "// SYNC: topcoat-ui-registry@{version} — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).\n"
    );

    // `Registry::names()` yields BTreeMap keys — already sorted.
    let names: Vec<String> = registry.names().map(String::from).collect();

    let mut count = 0;
    for name in &names {
        let component = registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("registry name {name} vanished between load and get"))?;
        let src = component.read_source()?;
        let dst_path = dst_dir.join(component.file_name());
        let mut content = format!("{header}{src}");
        // Patch separator.rs so composites can delegate without duplicating private logic
        if name == "separator" {
            content = content
                .replace(
                    "    fn classes(self) -> StaticClass {",
                    "    pub(crate) fn classes(self) -> StaticClass {",
                )
                .replace(
                    "    fn aria(self) -> Option<PromotedStr> {",
                    "    pub(crate) fn aria(self) -> Option<PromotedStr> {",
                );
        }
        if dry_run {
            println!("would sync {name} -> {}", dst_path.display());
        } else {
            std::fs::write(&dst_path, content)?;
            println!("synced {name}");
        }
        count += 1;
    }
    if dry_run {
        println!("dry-run: {count} components would be synced (topcoat-ui-registry@{version})");
    } else {
        println!(
            "done: {count} components synced to {} (topcoat-ui-registry@{version})",
            dst_dir.display()
        );
        println!("note: composites/ was not touched (ADR-0007)");
    }
    // Also ensure primitives/mod.rs lists every component in the registry
    ensure_primitives_mod(&dst_dir, &header, &names, dry_run)?;
    Ok(())
}

fn ensure_primitives_mod(
    dst_dir: &Path,
    header: &str,
    names: &[String],
    dry_run: bool,
) -> anyhow::Result<()> {
    let mod_path = dst_dir.join("mod.rs");
    let mut content = String::from(header);
    for name in names {
        content.push_str(&format!("pub mod {name};\n"));
    }
    if dry_run {
        println!("would write {}", mod_path.display());
    } else {
        std::fs::write(&mod_path, content)?;
        println!("wrote {}", mod_path.display());
    }
    Ok(())
}
