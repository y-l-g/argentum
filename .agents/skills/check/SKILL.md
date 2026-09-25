---
name: check
description: Always use this skill to verify a change locally before committing or opening a pull request in the Argentum repository
---

# Verifying a Change

Run the gates in [`CONTRIBUTING.md`](../../../CONTRIBUTING.md#the-gate-set): the ones
covering the touched area before pushing, and all ten before merging. That list mirrors
`.github/workflows/ci.yml` and is the canonical copy; the extra checks outside the ten
(docs, detached-bench fmt, bench-check) are listed there too.

The asset suites are named rather than globbed, exactly as the CI `assets` job
names them: a glob would silently shrink the run when a suite is renamed, while
a missing path fails the job. Gate 4's nightly date is recorded in
`rust-toolchain.toml`'s comment, so the nightly-only rustfmt keys `rustfmt.toml`
sets cannot move under the gate (GH #269).

Rules that catch the recurring failures:

- `topcoat fmt` only agrees with the CLI built from the rev `Cargo.lock` pins.
  Another CLI's diff is not a fix: install the locked rev (see
  [`CONTRIBUTING.md`](../../../CONTRIBUTING.md#the-topcoat-fmt-trap)) and run that.
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
  `cargo +nightly install cargo-udeps --locked`, then the udeps gate in
  [`CONTRIBUTING.md`](../../../CONTRIBUTING.md#the-gate-set).
- A gate whose command names a toolchain installs it on demand; gate 4's dated
  nightly install is the `rustup toolchain install` step of the `fmt` job in
  `.github/workflows/ci.yml`.
