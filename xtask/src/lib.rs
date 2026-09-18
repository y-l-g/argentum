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
    // Anchored at xtask's own manifest (GH #175): a bare `cargo metadata`
    // resolves the caller's CWD, so invoking from a detached workspace
    // (e.g. benchmarks/argentum, which has no topcoat-ui-registry in its
    // graph) failed with a misleading "must be a dependency of xtask".
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
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
///
/// `prune` deletes vendored files the registry no longer owns (the orphan
/// guard in `verify_sync` otherwise leaves `verify` red after an upstream
/// removal with `sync` alone unable to fix it). Without it, orphans are only
/// reported — pass `--prune` to converge.
pub fn sync_topcoat_ui(dry_run: bool, prune: bool) -> anyhow::Result<()> {
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
    if prune {
        prune_orphans(&dst_dir, &registry, dry_run)?;
    }
    Ok(())
}

/// Delete vendored files the registry manifest no longer owns (GH #175):
/// the same expected-set as the `verify_sync` orphan guard (`mod.rs`
/// included — it is regenerated, never pruned). Dry runs only report.
fn prune_orphans(dst_dir: &Path, registry: &Registry, dry_run: bool) -> anyhow::Result<()> {
    use std::collections::HashSet;
    let mut expected: HashSet<String> = HashSet::new();
    for name in registry.names() {
        if let Some(component) = registry.get(name) {
            expected.insert(component.file_name().to_string());
        }
    }
    expected.insert("mod.rs".to_string());
    let mut orphans: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dst_dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') {
                continue;
            }
            if !expected.contains(&file_name) {
                orphans.push(entry.path());
            }
        }
    }
    orphans.sort();
    for orphan in orphans {
        if dry_run {
            println!("would delete orphan {}", orphan.display());
        } else {
            std::fs::remove_file(&orphan)?;
            println!("deleted orphan {}", orphan.display());
        }
    }
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

    // Orphan guard (GH #103): an upstream-removed component must not linger as
    // a stale vendored file that still compiles when referenced. Flag any file
    // in primitives/ that the registry does not own.
    {
        use std::collections::HashSet;
        let mut expected_files: HashSet<String> = HashSet::new();
        for name in &names {
            if let Some(component) = registry.get(name) {
                expected_files.insert(component.file_name().to_string());
            }
        }
        expected_files.insert("mod.rs".to_string());
        if let Ok(entries) = std::fs::read_dir(&dst_dir) {
            let mut orphans: Vec<String> = Vec::new();
            for entry in entries.flatten() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with('.') {
                    continue;
                }
                if !expected_files.contains(&file_name) {
                    orphans.push(dst_dir.join(&file_name).display().to_string());
                }
            }
            orphans.sort();
            for orphan in orphans {
                failures.push(format!(
                    "{orphan} is not in the registry manifest (orphaned vendored file); delete it or {HINT}"
                ));
            }
        }
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

/// The directory holding the hand-written shell JS assets (ADR-0014).
pub fn assets_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // xtask is at <repo>/xtask, so repo root is parent of manifest_dir
    manifest_dir
        .parent()
        .unwrap_or(Path::new("."))
        .join("crates/argentum-ui/assets")
}

/// Shell JS assets (GH #152, ADR-0014): the file under `assets/` plus the
/// `argentum-ui` constant that wires it into the document head.
pub const ASSET_FILES: &[(&str, &str)] = &[
    ("sidebar.js", "SIDEBAR_JS"),
    ("theme.js", "THEME_JS"),
    ("dialog.js", "DIALOG_JS"),
    ("code_block.js", "CODE_BLOCK_JS"),
    ("bulk.js", "BULK_JS"),
    ("filters.js", "FILTERS_JS"),
    ("selects.js", "SELECTS_JS"),
    ("notifications.js", "NOTIFICATION_JS"),
];

/// One hook-contract entry (GH #152, ADR-0014): `js` must appear in the
/// asset's source and `rust` must appear somewhere in the Rust render sources
/// (`argentum-ui/src` + `argentum-core/src`, render sites and their tests).
/// Usually both are the same attribute hook; dataset-mapped hooks name each
/// side's spelling (`dialogOpenParam` reads `data-dialog-open-param`).
pub struct AssetHook {
    /// The asset file under `assets/` that consumes the hook.
    pub asset: &'static str,
    /// The needle that must appear in the asset's source.
    pub js: &'static str,
    /// The needle that must appear in the Rust sources.
    pub rust: &'static str,
}

/// The checked-in hook list (GH #152). Deliberately attribute hooks only —
/// structural selectors (`.relative`, `pre code`, `select option`,
/// `dialog[open]`, `#mobile-sidebar-sheet`, which has no JS consumer: the
/// sheet backdrop is a runtime `@click` handler) and the inverse direction (a
/// rendered hook with no consumer, e.g. `data-bulk-ids`) are out of scope, as
/// are generic storage keys (`theme`, whose substring matches everything).
/// Track new hooks here as they land.
pub const ASSET_HOOKS: &[AssetHook] = &[
    AssetHook {
        asset: "sidebar.js",
        js: "data-sidebar",
        rust: "data-sidebar",
    },
    AssetHook {
        asset: "sidebar.js",
        js: "data-state",
        rust: "data-state",
    },
    AssetHook {
        asset: "sidebar.js",
        js: "sidebar_state",
        rust: "sidebar_state",
    },
    AssetHook {
        asset: "theme.js",
        js: "data-theme-toggle",
        rust: "data-theme-toggle",
    },
    AssetHook {
        asset: "dialog.js",
        js: "data-dialog-close",
        rust: "data-dialog-close",
    },
    AssetHook {
        asset: "dialog.js",
        js: "dialogOpenParam",
        rust: "data-dialog-open-param",
    },
    AssetHook {
        asset: "code_block.js",
        js: "data-copy-button",
        rust: "data-copy-button",
    },
    AssetHook {
        asset: "bulk.js",
        js: "data-bulk-form",
        rust: "data-bulk-form",
    },
    AssetHook {
        asset: "bulk.js",
        js: "data-table-root",
        rust: "data-table-root",
    },
    AssetHook {
        asset: "bulk.js",
        js: "data-bulk-submit",
        rust: "data-bulk-submit",
    },
    AssetHook {
        asset: "bulk.js",
        js: "data-row-select",
        rust: "data-row-select",
    },
    AssetHook {
        asset: "bulk.js",
        js: "data-bulk-select-all",
        rust: "data-bulk-select-all",
    },
    AssetHook {
        asset: "bulk.js",
        js: "name=\"ids\"",
        rust: "name=\"ids\"",
    },
    AssetHook {
        asset: "filters.js",
        js: "data-filter-name",
        rust: "data-filter-name",
    },
    AssetHook {
        asset: "filters.js",
        js: "data-filters-form",
        rust: "data-filters-form",
    },
    AssetHook {
        asset: "filters.js",
        js: "data-filters-transport",
        rust: "data-filters-transport",
    },
    AssetHook {
        asset: "filters.js",
        js: "data-filters-live",
        rust: "data-filters-live",
    },
    AssetHook {
        asset: "selects.js",
        js: "data-select-filterable",
        rust: "data-select-filterable",
    },
    AssetHook {
        asset: "selects.js",
        js: "data-options-filter",
        rust: "data-options-filter",
    },
    AssetHook {
        asset: "notifications.js",
        js: "data-sonner-toast",
        rust: "data-sonner-toast",
    },
    AssetHook {
        asset: "notifications.js",
        js: "data-close-button",
        rust: "data-close-button",
    },
    AssetHook {
        asset: "notifications.js",
        js: "dataset.mounted",
        rust: "data-mounted",
    },
];

/// Whether `needle` appears in `haystack` as a hook, not as a prefix of a
/// longer name.
///
/// A plain substring check misses renames by extension (`data-copy-button` →
/// `data-copy-button-2` still contains the old string), so an occurrence only
/// counts when neither neighbor continues the name. Still structural: any
/// spelling (`[data-x]`, `data-x=""`, `dataset.x`) matches.
fn contains_hook(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.match_indices(needle).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        !before.is_some_and(is_hook_char) && !after.is_some_and(is_hook_char)
    })
}

/// Characters that continue a hook/identifier name.
fn is_hook_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// What to do when the hook contract breaks.
const HOOK_HINT: &str = "update the hook list and both sides together (GH #152, ADR-0014)";

/// Collect every `.rs` file under `dir`, recursively.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Guard: every shell JS asset still exists and stays wired to its `lib.rs`
/// constant, and every hook in [`ASSET_HOOKS`] still appears in both its JS
/// asset and the Rust render sources.
///
/// This is the hook-contract counterpart of [`verify_sync`]: `asset!` does
/// not stat its source at compile time, so a deleted/renamed `.js` passes
/// `cargo test`, and a rename on either side of a string-selector coupling is
/// otherwise silent. The check is structural on purpose (hook-name presence
/// with identifier-boundary matching, never classes or pixel markup) so
/// restyles cannot fail it.
pub fn verify_asset_hooks() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent().unwrap_or(Path::new("."));
    let assets = assets_dir();
    let mut failures = Vec::new();

    let lib_rs = std::fs::read_to_string(root.join("crates/argentum-ui/src/lib.rs"))
        .map_err(|error| anyhow::anyhow!("cannot read argentum-ui/src/lib.rs: {error}"))?;

    // Every asset file exists and stays wired to its constant.
    let mut sources: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for (file, constant) in ASSET_FILES {
        let path = assets.join(file);
        match std::fs::read_to_string(&path) {
            Ok(src) if !src.trim().is_empty() => {
                sources.insert(file, src);
            }
            Ok(_) => failures.push(format!("{file} is empty; {HOOK_HINT}")),
            Err(error) => failures.push(format!(
                "{} cannot be read: {error}; {HOOK_HINT}",
                path.display()
            )),
        }
        if !contains_hook(&lib_rs, constant) {
            failures.push(format!(
                "{constant} is gone from argentum-ui/src/lib.rs, so {file} is no longer wired into the document; {HOOK_HINT}"
            ));
        }
    }

    // Every hook appears in both its JS asset and the Rust sources.
    let mut rust_sources = String::new();
    let mut rs_files = Vec::new();
    for dir in ["crates/argentum-ui/src", "crates/argentum-core/src"] {
        if let Err(error) = collect_rs(&root.join(dir), &mut rs_files) {
            failures.push(format!("cannot list {dir}: {error}; {HOOK_HINT}"));
        }
    }
    for path in &rs_files {
        match std::fs::read_to_string(path) {
            Ok(src) => {
                rust_sources.push_str(&src);
                rust_sources.push('\n');
            }
            Err(error) => failures.push(format!(
                "{} cannot be read: {error}; {HOOK_HINT}",
                path.display()
            )),
        }
    }
    for hook in ASSET_HOOKS {
        match sources.get(hook.asset) {
            Some(src) if contains_hook(src, hook.js) => {}
            Some(_) => failures.push(format!(
                "{} no longer contains `{}`; {HOOK_HINT}",
                hook.asset, hook.js
            )),
            None => failures.push(format!(
                "{} is missing, so its `{}` hook cannot be checked; {HOOK_HINT}",
                hook.asset, hook.js
            )),
        }
        if !contains_hook(&rust_sources, hook.rust) {
            failures.push(format!(
                "`{}` (consumed by {}) is gone from the Rust sources; {HOOK_HINT}",
                hook.rust, hook.asset
            ));
        }
    }

    if failures.is_empty() {
        println!(
            "verified: {} assets and {} hooks match the hook contract",
            ASSET_FILES.len(),
            ASSET_HOOKS.len()
        );
        Ok(())
    } else {
        anyhow::bail!("asset hook drift detected:\n{}", failures.join("\n"));
    }
}
