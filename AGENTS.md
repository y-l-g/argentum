# Argentum — Agent Instructions

## Commands

```sh
# The gate set (.github/workflows/ci.yml). All nine before merging.
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check -p argentum-core --no-default-features --locked
cargo fmt -p argentum-core -p argentum-macros -p argentum-ui -p showcase -p xtask -- --check
topcoat fmt && git diff --exit-code
cargo check --locked --manifest-path benchmarks/argentum/Cargo.toml
cargo clippy --locked --manifest-path benchmarks/argentum/Cargo.toml --all-targets -- -D warnings
cargo +1.98 check --workspace --locked           # MSRV floor (rust-version 1.98)
node --test crates/argentum-ui/assets/selects.test.js crates/argentum-ui/assets/bulk.test.js \
  crates/argentum-ui/assets/dialog.test.js crates/argentum-ui/assets/mutation-submit.test.js \
  examples/showcase/assets/media.test.js

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

| Path | Contents |
| --- | --- |
| `crates/argentum-core` | Panel, Resource, Table, Schema, auth, tenancy |
| `crates/argentum-macros` | the `EmbeddedForm` derive |
| `crates/argentum-ui` | vendored primitives + owned composites |
| `examples/showcase` | runnable admin + integration tests |
| `benchmarks/` | `argentum`, `axum-maud`, `leptos` — detached workspaces |
| `README.md` | entry point; the user guide is `docs/guide` (mdBook) |
| `CONTEXT.md`, `docs/adr` | domain vocabulary, decisions |
| `docs/dev` | specs (commits, prose, labels, testing) and `architecture.md` |
| `docs/agents` | tracker notes for agents |
| `.agents/skills` | load-when instructions: `check`, `issue`, `pr`, `prose`, `style` |

## Renovate PRs

- Never blanket `cargo update`: `topcoat`/`toasty` track `main`. Bump with `cargo update -p
  topcoat -p toasty`, then `cargo check --offline`; update each green bot branch onto `master`,
  verify, merge.
- Coupled or breaking sets (e.g. `argon2` + `password-hash`) merge as one combined manual bump,
  verified once; close the bot PRs as superseded.
- Every bump touching the workspace lock syncs `benchmarks/argentum/Cargo.lock` in the same
  commit (GH #103).
- Two `syn` majors remain (GH #181), not because of topcoat: the #193 bump moved every
  `topcoat-*-macro`/`-grammar` crate to `syn 3`. The `syn 2` half is 24 crates-io proc-macro
  crates nobody here controls, so it does not clear when any one moves; do not force-unify.

## Further reading

`CONTRIBUTING.md` · `docs/dev/architecture.md` · `docs/dev/COMMITS.md` · `docs/dev/PROSE.md` ·
`docs/dev/LABELS.md` · `docs/dev/TESTING.md` · `docs/dev/design/` ·
`docs/dev/upstream-notes.md` · `docs/guide/` · `CONTEXT.md` · `docs/adr/` · `docs/agents/`.
