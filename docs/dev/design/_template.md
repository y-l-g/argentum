<!--
Design document template.

Copy this file to `docs/dev/design/<feature-name>.md` and fill it in.
Delete sections that genuinely do not apply (and explain why in one line),
but keep the section order.

A design document is guide-level: write it for the people who will use the
change, not for someone reading the implementation. The two audiences are:

  - App developers — developers building an admin with Tablo.
  - Toolkit contributors — anyone extending Panel, Resource, Table, or Schema.

Describe what those audiences will see and have to do. Avoid documenting
internal modules or implementation choices that have no observable effect.
-->

# {Feature name}

Closes #{issue number}.

## Summary

One paragraph: what changes for app developers, and why this is worth doing.

## Motivation

The problem this solves. Concrete admin scenarios that are awkward,
impossible, or surprising today. Quote real issues if you have them.

## User-facing API

Write this section as a chapter of the user guide — prose that can be adapted
into `docs/guide/src/` once the feature ships. Introduce the concept, then show
idiomatic use with code examples. Tell the reader what to call, when to reach
for it, and how it fits with features they already know.

Code blocks here are illustrative only — they do not need to compile and are
not tested. Do **not** add doctest boilerplate (`# use …`,
`# async fn __example(…) { … }`, etc.); show only the lines that matter.

When this changes existing API, include a short "Before and after" showing how
app code migrates.

## Behavior

How the API behaves at runtime. Include:

- Happy path — what the developer gets back.
- Error cases — what the developer sees when things go wrong, and what type of
  error it is.
- Defaults and implicit behavior the developer does not control.
- Interactions with other features (policy checks, tenancy scoping, exports,
  pagination, transactions — whichever apply).

## Edge cases

Cases that affect app code and are easy to get wrong:

- Boundary values, empty inputs, null handling.
- Concurrency or ordering assumptions.
- Authorization and tenancy interactions: what a caller without access sees.

## Alternatives

Other approaches weighed and why they were discarded.

## Open questions

Split into blocking-acceptance (must resolve before the design merges),
blocking-implementation (must resolve before code lands), and deferrable.

## Out of scope

What this design deliberately does not cover, so reviewers do not expand it.
