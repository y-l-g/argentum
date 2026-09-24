---
name: check
description: Always use this skill to verify a change locally before committing or opening a pull request in the Argentum repository
---

# Verifying a Change

Keep this file in sync with `.github/workflows/ci.yml`.

Run the gates covering the touched area before pushing, and all ten before merging:

```
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p argentum-core --no-default-features --locked
cargo +nightly fmt --all -- --check
topcoat fmt && git diff --exit-code
cargo check --locked --manifest-path benchmarks/argentum/Cargo.toml
cargo clippy --locked --manifest-path benchmarks/argentum/Cargo.toml --all-targets -- -D warnings
cargo +1.98 check --workspace --locked
node --test crates/argentum-ui/assets/selects.test.js crates/argentum-ui/assets/bulk.test.js \
  crates/argentum-ui/assets/dialog.test.js crates/argentum-ui/assets/mutation-submit.test.js \
  examples/showcase/assets/media.test.js
cargo +nightly udeps --workspace --all-targets --all-features --locked
```

The asset suites are named rather than globbed, exactly as the CI `assets` job
names them: a glob would silently shrink the run when a suite is renamed, while
a missing path fails the job.

Rules that catch the recurring failures:

- `topcoat fmt` only agrees with the CLI built from the rev `Cargo.lock` pins.
  Another CLI's diff is not a fix: install the locked rev (see `CONTRIBUTING.md`)
  and run that.
- `cargo fmt` covers workspace members only; the detached `benchmarks/*`
  workspaces are formatted and linted by manifest path.
- Any lockfile change syncs `benchmarks/argentum/Cargo.lock` in the same commit,
  with identical `topcoat`/`toasty` revs.
- Give each worktree its own target directory; a shared `CARGO_TARGET_DIR`
  cross-contaminates.
- Never pipe when you need the exit code: `| tail` masks it. Read `PIPESTATUS`
  or redirect to a file.
- Never hand-edit `crates/argentum-ui/src/components/primitives/`; sync it with
  `cargo xtask sync-topcoat-ui`.
- `cargo udeps` needs `cargo-udeps` on nightly for `-Z binary-dep-depinfo`:
  `cargo +nightly install cargo-udeps --locked`, then the gate command above.
