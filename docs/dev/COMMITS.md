# Commit messages

Every branch is squash-merged into `master`: one commit per branch, so the
history has no empty merge commits. A branch's individual commits are working
notes.

The squashed commit is a Conventional Commit. Keep the issue reference in the
subject — it is what links the history back to the tracker.

```
<type>(<scope>): <description> (#123)
```

## Types

`feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `chore`, `build`, `ci`.

## Scope

The subsystem the change touches: `table`, `panel`, `schema`, `core`, `ui`,
`xtask`, `showcase`, `docs`, `repo`.

Several issues list them all:

```
fix(table): bound the filters signal (#205, #219)
```

## Breaking changes

Mark a breaking change with `!` after the type or the scope, and explain it in a
`BREAKING CHANGE:` footer:

```
feat(schema)!: choose an embedded enum's variant in the form (#191)

BREAKING CHANGE: an embedded enum's discriminant is now visible, and
`discriminant_input` / the hidden control are replaced by `discriminant_select`.
```

## Body

Explain what changed and why it changed. Do not restate the diff, and do not
narrate the path that produced it — describe the change the commit makes.
Write the body per [`PROSE.md`](PROSE.md): current behavior, active voice,
no filler.
