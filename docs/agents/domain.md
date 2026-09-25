# Domain docs

This is a single-context repo: one `CONTEXT.md` at the root holds the domain
vocabulary, and `docs/adr/` holds the decisions. There is no `CONTEXT-MAP.md`
and no per-crate context.

Read `CONTEXT.md` and the ADRs that touch the area you are about to work in
before exploring the code.

## Use the glossary's vocabulary

Name a domain concept with the term `CONTEXT.md` defines — in an issue title, a
refactor proposal, a test name, or a code identifier. Do not drift to a synonym
its `_Avoid_` line lists. If the concept you need is missing from the glossary,
say so rather than inventing a term.

## Flag ADR conflicts

If your output contradicts an ADR, surface it explicitly rather than silently
overriding it:

> Contradicts ADR-0007 (primitives vs composites in `tablo-ui`), but worth
> reopening because…
