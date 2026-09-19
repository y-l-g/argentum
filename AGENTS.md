# Argentum — Agent Instructions

## Agent skills

### Issue tracker

Issues live as GitHub issues in y-l-g/argentum (via `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles map 1:1 to `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Git

Land branches fast-forward when `master` hasn't diverged (no empty merge commits); use `--no-ff` only for true merges.

## Renovate PRs

- Never blanket `cargo update`: `topcoat`/`toasty` track `main` and bump deliberately, and v3 breaks `topcoat fmt`, so a workspace-wide `syn` bump stays off the table.
- The graph carries **two `syn` majors on purpose** (GH #181): `argentum-macros` is on `syn 3`, while every `topcoat-*-macro` crate pins `^2.0.117` upstream. Don't force-unify them — the duplicate clears only when topcoat moves.
- Green patches: update each bot branch onto `master`, verify, merge.
- Coupled/breaking sets (e.g. `argon2` + `password-hash`): land as one combined manual bump, verify once, close the bot PRs as superseded.
- Every bump touching the workspace lock must sync `benchmarks/argentum/Cargo.lock` in the same commit (pin policy GH #103) and keep topcoat/toasty revs identical across both lockfiles.
