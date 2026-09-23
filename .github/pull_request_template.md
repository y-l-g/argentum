## Summary

What changed and why. Link the issue (`Closes #123`). If an AI agent did the
work, name the model and describe what it did.

## Verification

The gates that ran and their result, e.g. `cargo test --workspace --locked`.
Name anything not run and why. The full set is in `CONTRIBUTING.md`.

## Checklist

- [ ] The gate set for the touched area passes.
- [ ] If a lockfile changed, `benchmarks/argentum/Cargo.lock` is synced in this commit and the `topcoat`/`toasty` revs match.
- [ ] If `view!` markup changed, `topcoat fmt` ran with the CLI built from the locked rev.
- [ ] If a doc claim changed, it was verified against the code.
- [ ] No history or narrative in code comments or docs (`docs/dev/PROSE.md`).
