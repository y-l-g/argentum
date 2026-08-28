use std::path::{Path, PathBuf};

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
    sync-topcoat-ui    Copy verbatim primitives from topcoat-ui-registry
                       (../topcoat/crates/topcoat-ui/registry/src/components/*.rs)
                       into crates/argentum-ui/src/components/primitives/*.rs
                       with SYNC header. Never touches composites/.

OPTIONS:
    --dry-run          Print what would be copied without writing
    --help, -h         Show this help
"#
    );
}

fn sync_topcoat_ui(dry_run: bool) -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // xtask is at <repo>/xtask, so repo root is parent of manifest_dir
    let repo_root = manifest_dir.parent().unwrap_or(Path::new("."));
    let src_dir = repo_root
        .join("../topcoat/crates/topcoat-ui/registry/src/components");
    let dst_dir = repo_root.join("crates/argentum-ui/src/components/primitives");

    if !src_dir.exists() {
        anyhow::bail!(
            "source not found: {} (expected sibling ../topcoat)",
            src_dir.display()
        );
    }
    if !dst_dir.exists() {
        std::fs::create_dir_all(&dst_dir)?;
    }

    // Compute upstream commit for SYNC header — `git -C ../topcoat rev-parse --short HEAD`
    let commit = std::process::Command::new("git")
        .args(["-C", &repo_root.join("../topcoat").to_string_lossy().to_string(), "rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { None })
        .unwrap_or_else(|| "main".to_string());
    let header = format!("// SYNC: topcoat-ui-registry@{commit} — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).\n");

    let mut count = 0;
    for entry in std::fs::read_dir(&src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let dst_path = dst_dir.join(&file_name);
        let src_content = std::fs::read_to_string(&path)?;
        let mut dst_content = format!("{header}{src_content}");
        // Patch separator.rs so composites can delegate without duplicating private logic
        if file_name == "separator.rs" {
            dst_content = dst_content
                .replace("    fn classes(self) -> StaticClass {", "    pub(crate) fn classes(self) -> StaticClass {")
                .replace("    fn aria(self) -> Option<PromotedStr> {", "    pub(crate) fn aria(self) -> Option<PromotedStr> {");
        }
        if dry_run {
            println!("would sync {file_name} -> {}", dst_path.display());
        } else {
            std::fs::write(&dst_path, dst_content)?;
            println!("synced {file_name}");
        }
        count += 1;
    }
    if dry_run {
        println!("dry-run: {count} files would be synced (header @{commit})");
    } else {
        println!("done: {count} files synced to {} (header @{commit})", dst_dir.display());
        println!("note: composites/ was not touched (ADR-0007)");
    }
    // Also ensure primitives/mod.rs lists all files
    ensure_primitives_mod(&dst_dir, &header, dry_run)?;
    Ok(())
}

fn ensure_primitives_mod(dst_dir: &Path, header: &str, dry_run: bool) -> anyhow::Result<()> {
    let mod_path = dst_dir.join("mod.rs");
    let mut files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dst_dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        if stem == "mod" {
            continue;
        }
        files.push(stem);
    }
    files.sort();
    let mut content = String::from(header);
    for stem in files {
        content.push_str(&format!("pub mod {stem};\n"));
    }
    if dry_run {
        println!("would write {}", mod_path.display());
    } else {
        std::fs::write(&mod_path, content)?;
        println!("wrote {}", mod_path.display());
    }
    Ok(())
}
