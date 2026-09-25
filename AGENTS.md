# Argentum — Agent Instructions

## Commands

The gate set lives in [`CONTRIBUTING.md`](CONTRIBUTING.md#the-gate-set): ten commands
mirroring `.github/workflows/ci.yml`. Run the ones covering your change, all ten before
merging. CI also runs the extra checks listed there (docs, detached-bench fmt, bench-check).

```sh
cargo run -p showcase                            # http://localhost:3000/admin/users
cargo xtask sync-topcoat-ui                      # re-vendor primitives, verbatim
cargo xtask verify-topcoat-ui                    # fail on vendored drift

# `topcoat fmt` only agrees with the CLI built from the rev Cargo.lock pins.
REV=$(grep -A 2 '^name = "topcoat"$' Cargo.lock | grep -o '#[0-9a-f]\{40\}' | head -1 | cut -c2-)
cargo install --git https://github.com/tokio-rs/topcoat --rev "$REV" topcoat-cli --locked
```

## Rules

1. Verify every factual claim in a doc, comment, or commit message against the code.
2. Document current behavior only; no "used to", "previously". See `docs/dev/PROSE.md`.
3. Run the gate set for the area you touched, plus `cargo test --workspace --locked` on the
   merged result: branches can merge cleanly and not compile.
4. Give each worktree its own target directory; a shared `CARGO_TARGET_DIR` cross-contaminates.
5. Never pipe when you need the exit code: `| tail` masks it. Read `PIPESTATUS` or redirect to
   a file.
6. `cargo fmt` covers workspace members only; the detached `benchmarks/*` workspaces are
   formatted and linted by manifest path.
7. Any lockfile change syncs `benchmarks/argentum/Cargo.lock` in the same commit, with
   identical `topcoat`/`toasty` revs.
8. Never hand-edit `crates/argentum-ui/src/components/primitives/`; sync it with xtask. Owned
   components live in `components/composites/`.
9. Hunting dead code: prefer `pub` API, always-same-value config, and test-only paths.
   `unsafe_code` and `warnings` are denied; `too_many_lines` is allowed.
10. Run `topcoat fmt` with the locked-rev CLI after changing `view!` markup; another CLI's
    diff is not a fix. See `CONTRIBUTING.md`.

## Git

Squash-merge every branch into `master` — one commit per branch, no empty merge commits; a
branch's commits are working notes. The squashed commit is a Conventional Commit carrying the
issue in the subject: `<type>(<scope>): <description> (#123)` (`docs/dev/COMMITS.md`).

## Layout

Crate roles live in [`docs/dev/architecture.md`](docs/dev/architecture.md#crates). The user
guide is `docs/guide/` (mdBook), decisions are in `docs/adr/`, contributor specs in
`docs/dev/`, domain vocabulary in `CONTEXT.md`, and agent tracker notes in `docs/agents/`.

## Renovate PRs

Bump `topcoat`/`toasty` deliberately, never with a blanket `cargo update`; sync
`benchmarks/argentum/Cargo.lock` in the same commit. Coupled sets (e.g. `argon2` +
`password-hash`) merge as one combined manual bump. See
[`CONTRIBUTING.md`](CONTRIBUTING.md#dependency-pins) for the commands and GH #103.
Two `syn` majors remain (GH #181, GH #193); do not force-unify.

## Further reading

- Build and verify: [`CONTRIBUTING.md`](CONTRIBUTING.md), `docs/dev/architecture.md`,
  `docs/dev/TESTING.md`
- Write: `docs/dev/PROSE.md`, `docs/dev/COMMITS.md`, `docs/dev/LABELS.md`, `docs/guide/`,
  `CONTEXT.md`
- Decide: `docs/adr/`, `docs/dev/design/`, `docs/dev/upstream-notes.md`, `docs/agents/`.
