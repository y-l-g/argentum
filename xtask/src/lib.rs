//! xtask — repo tasks (ADR-0007).
//!
//! `sync-topcoat-ui` mirrors the `topcoat-ui-registry` sources into
//! `crates/argentum-ui/src/components/primitives/` **verbatim**: every file is
//! the registry's byte-for-byte source under a one-line SYNC header that
//! records the registry version *and* the sha256 content hash of the source
//! (the same hash scheme topcoat's own registry and `topcoat ui` use). Because
//! the copy is verbatim, drift — a hand edit, a stale file, a component the
//! registry gained or dropped — is detectable by [`verify_sync`], which the
//! `xtask` test suite runs as a guard.

use std::path::{Path, PathBuf};

use topcoat_ui::Registry;

/// The one-line header prepended to every synced file.
///
/// `hash` is the registry source's `sha256:` content hash (see
/// `topcoat_ui::content_hash`), so a guard can tell a drifted file
/// from a merely stale header without a sibling clone.
fn sync_header(version: &str, hash: &str) -> String {
    format!(
        "// SYNC: topcoat-ui-registry@{version} {hash} — do not hand-edit. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).\n"
    )
}

/// The header for the generated `mod.rs` (it is not a copy of any source, so
/// it carries only the registry version).
fn mod_header(version: &str) -> String {
    format!(
        "// SYNC: topcoat-ui-registry@{version} — generated from the registry manifest. Sync via `cargo xtask sync-topcoat-ui` (ADR-0007).\n"
    )
}

/// The destination directory for synced primitives.
pub fn primitives_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // xtask is at <repo>/xtask, so repo root is parent of manifest_dir
    manifest_dir
        .parent()
        .unwrap_or(Path::new("."))
        .join("crates/argentum-ui/src/components/primitives")
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

/// What to run when a vendored file has drifted from the registry.
const HINT: &str = "run `cargo xtask sync-topcoat-ui` to restore the verbatim copy";

/// Copy every registry component into `primitives/` **verbatim** under a SYNC
/// header recording the registry version and the source's sha256, then
/// regenerate `mod.rs` from the manifest. Never touches `composites/`
/// (ADR-0007).
///
/// No sibling clone required — the registry comes from the same git source
/// Cargo compiles against.
pub fn sync_topcoat_ui(dry_run: bool) -> anyhow::Result<()> {
    let dst_dir = primitives_dir();
    std::fs::create_dir_all(&dst_dir)?;

    let (registry, version) = locate_registry()?;

    // `Registry::names()` yields BTreeMap keys — already sorted.
    let names: Vec<String> = registry.names().map(String::from).collect();

    let mut count = 0;
    for name in &names {
        let component = registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("registry name {name} vanished between load and get"))?;
        let src = component.read_source()?;
        let header = sync_header(&version, &topcoat_ui::content_hash(&src));
        let dst_path = dst_dir.join(component.file_name());
        if dry_run {
            println!("would sync {name} -> {}", dst_path.display());
        } else {
            std::fs::write(&dst_path, format!("{header}{src}"))?;
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
    ensure_primitives_mod(&dst_dir, &version, &names, dry_run)?;
    Ok(())
}

/// Regenerate `primitives/mod.rs` from the registry manifest.
fn ensure_primitives_mod(
    dst_dir: &Path,
    version: &str,
    names: &[String],
    dry_run: bool,
) -> anyhow::Result<()> {
    let mod_path = dst_dir.join("mod.rs");
    let mut content = mod_header(version);
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

/// Guard: every vendored primitive is still the registry's verbatim source,
/// every SYNC header records the current version *and* the current content
/// hash, and `mod.rs` still lists exactly the registry's components.
///
/// This is Argentum's counterpart of topcoat's own
/// `examples/ui/tests/registry_sync.rs`: because the sync is byte-for-byte
/// (no injected headers *inside* the source, no string patches), a hash
/// comparison is meaningful and drift cannot hide.
pub fn verify_sync() -> anyhow::Result<()> {
    let dst_dir = primitives_dir();
    let (registry, version) = locate_registry()?;

    let names: Vec<String> = registry.names().map(String::from).collect();
    let mut failures = Vec::new();

    for name in &names {
        let component = registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("registry name {name} vanished between load and get"))?;
        let src = component.read_source()?;
        let expected = sync_header(&version, &topcoat_ui::content_hash(&src)) + &src;
        let dst_path = dst_dir.join(component.file_name());
        let installed = match std::fs::read_to_string(&dst_path) {
            Ok(content) => content,
            Err(error) => {
                failures.push(format!(
                    "{} cannot be read: {error}; {HINT}",
                    dst_path.display()
                ));
                continue;
            }
        };
        if installed == expected {
            continue;
        }
        // Distinguish a stale/mismatched header from a hand edit of the body.
        let header_matches = installed
            .strip_prefix("// SYNC: topcoat-ui-registry@")
            .is_some_and(|rest| {
                rest.split_once(" — do not hand-edit.")
                    .is_some_and(|(head, _)| {
                        head.split_once(' ').is_some_and(|(ver, hash)| {
                            ver == version && hash == topcoat_ui::content_hash(&src)
                        })
                    })
            });
        if header_matches {
            failures.push(format!(
                "{} was hand-edited — it no longer matches the registry source; {HINT}",
                dst_path.display()
            ));
        } else {
            failures.push(format!(
                "{} carries a stale SYNC header or drifted content (topcoat-ui-registry@{version}); {HINT}",
                dst_path.display()
            ));
        }
    }

    // mod.rs must list exactly the registry's components.
    let mod_path = dst_dir.join("mod.rs");
    let mut expected = mod_header(&version);
    for name in &names {
        expected.push_str(&format!("pub mod {name};\n"));
    }
    match std::fs::read_to_string(&mod_path) {
        Ok(actual) if actual == expected => {}
        Ok(_) => failures.push(format!(
            "{} does not match the registry manifest; {HINT}",
            mod_path.display()
        )),
        Err(error) => failures.push(format!(
            "{} cannot be read: {error}; {HINT}",
            mod_path.display()
        )),
    }

    if failures.is_empty() {
        println!(
            "verified: {} primitives match topcoat-ui-registry@{version} verbatim",
            names.len()
        );
        Ok(())
    } else {
        anyhow::bail!("registry drift detected:\n{}", failures.join("\n"));
    }
}
