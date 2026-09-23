---
name: commit
description: Always use this skill before authoring a commit message in the Argentum repository
---

# Authoring Commit Messages

Read [`docs/dev/COMMITS.md`](../../../docs/dev/COMMITS.md) before authoring a
commit message. It is the authoritative source for the format, allowed types,
scope conventions, subject/body/footer rules, and breaking-change notation.

Branches squash-merge into `master` — one Conventional Commit per branch, with
the issue reference in the subject: `<type>(<scope>): <description> (#123)`.
PR titles follow the same format (enforced by the `semantic-pr` workflow) since
the title becomes the landed commit.

## Be succinct

Maintainers already know Argentum and Rust. State what changed and why; skip
restated context and throat-clearing.
