---
name: issue
description: Always use this skill before opening an issue in the Argentum repository
---

# Opening Issues

Write per [`PROSE.md`](../../../docs/dev/PROSE.md): current behavior, active
voice, no filler. State what happened or what is proposed, not how important
it is.

## Pick the right template

Issue templates live in [`.github/ISSUE_TEMPLATE/`](../../../.github/ISSUE_TEMPLATE):

- [`bug_report.yml`](../../../.github/ISSUE_TEMPLATE/bug_report.yml) — a defect
  in the framework, the showcase, or the docs. Reports what was done, what was
  expected, and what actually happened (quote the exact message, never
  paraphrase it), plus a minimal reproduction. A failing test under
  `examples/showcase/tests/` is ideal.
- [`feature_proposal.yml`](../../../.github/ISSUE_TEMPLATE/feature_proposal.yml) —
  a new feature or public-API change. Leads with the problem and who hits it,
  sketches the concrete API, lists alternatives considered and a scope estimate.
- [`upstream_gap.yml`](../../../.github/ISSUE_TEMPLATE/upstream_gap.yml) — a
  missing or unstable Toasty/Topcoat API forcing an Argentum workaround. One gap
  per issue. The body is the status: edit it when the upstream state changes,
  never discuss status in comments.

Read the template before writing and fill in every field. If a field does not
apply, say so explicitly rather than leaving it blank.

## Bug reports

Report what you observed, not why you think it happened. A guess at the cause
sends triage down the wrong path; diagnosing is the maintainer's job. The
reproducer is the single most useful part of the report.

## Labels

Do not apply labels when creating the issue. The templates set the initial
label; maintainers triage the rest. See
[`docs/dev/LABELS.md`](../../../docs/dev/LABELS.md).
