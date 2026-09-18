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

- Never blanket `cargo update`: `syn` is pinned `<3` (v3 breaks topcoat fmt), and `topcoat`/`toasty` track `main` and bump deliberately.
- Green patches: update each bot branch onto `master`, verify, merge.
- Coupled/breaking sets (e.g. `argon2` + `password-hash`): land as one combined manual bump, verify once, close the bot PRs as superseded.
- Every bump touching the workspace lock must sync `benchmarks/argentum/Cargo.lock` in the same commit (pin policy GH #103) and keep topcoat/toasty revs identical across both lockfiles.
