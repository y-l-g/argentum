---
name: pr
description: Always use this skill before opening a pull request in the Tablo repository
---

# Opening Pull Requests

## Describe the diff, not the latest commit

A branch usually holds several commits: initial work, fixups, review responses,
rebases. The title and body describe the net change landing on `master`. Read
the full diff first:

```
git diff master...HEAD
git log master..HEAD
```

## Title

Same Conventional Commits format as
[`COMMITS.md`](../../../docs/dev/COMMITS.md), with the issue reference in the
subject:
`<type>(<scope>): <description> (#123)`.
PRs are squash-merged, so the title becomes the landed commit; reviewers check it.

## Body

Fill in the template at
[`.github/pull_request_template.md`](../../../.github/pull_request_template.md):
what changed and why with the issue link (`Closes #123`), the gates that ran
and their result, and the checklist. Keep the section headings; delete checklist
items that do not apply rather than leaving them unchecked with no explanation.

If an AI agent created the change, name the model and describe what it did.

## Be succinct

Reviewers already know Tablo and Rust. Include what they need to evaluate the
change, and nothing else.
