# Argentum — Agent Instructions

## Agent skills

### Issue tracker

Issues live as GitHub issues in y-l-g/argentum (via `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles map 1:1 to `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Git

**Squash-merge every branch into `master`** — one commit per branch, so no empty merge commits. A branch's individual commits are working notes; they do not belong in the history.

Write the squashed commit as a **Conventional Commit**, keeping the issue reference in the subject (that is what links the history back to the tracker):

```
<type>(<scope>): <description> (#123)
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `chore`, `build`, `ci`. Mark a breaking change with `!` after the type or scope and explain it in a `BREAKING CHANGE:` footer. Prefer the subsystem as the scope (`table`, `panel`, `schema`, `core`, `ui`, `xtask`, `showcase`).

A branch addressing several issues lists them all: `fix(table): bound the filters signal (#205, #219)`.

## Renovate PRs

- Never blanket `cargo update`: `topcoat`/`toasty` track `main` and bump deliberately. Bump with `cargo update -p topcoat -p toasty`, then `cargo check --offline` — the offline check proves the new revs resolve from the local git cache, so a failed fetch cannot quietly leave the lock half-updated. One command, both sources: see the workspace `Cargo.toml` comment.
- The graph still carries **two `syn` majors** (GH #181), but no longer because of topcoat: the #193 bump moved every `topcoat-*-macro`/`-grammar` crate to `syn 3` alongside `argentum-macros` and `toasty-macros`. The `syn 2` half is now 24 crates-io proc-macro crates nobody here controls (`bindgen`, `prost-derive`, `zerocopy-derive`, `windows-*`, `turso_*`, `tracing-attributes`, …), so the duplicate does **not** clear when any one of them moves. Don't force-unify.
- Green patches: update each bot branch onto `master`, verify, merge.
- Coupled/breaking sets (e.g. `argon2` + `password-hash`): land as one combined manual bump, verify once, close the bot PRs as superseded.
- Every bump touching the workspace lock must sync `benchmarks/argentum/Cargo.lock` in the same commit (pin policy GH #103) and keep topcoat/toasty revs identical across both lockfiles.
