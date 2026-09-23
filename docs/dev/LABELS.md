# Labels

The issue-tracker vocabulary. The three issue forms apply their own label;
everything else is applied by a maintainer.

## Applied by a template

| Label | Form | Meaning |
| --- | --- | --- |
| `bug` | Bug report | Something is not working. |
| `enhancement` | Feature proposal | A new feature or a change to the public API. |
| `upstream` | Upstream gap | Missing or unstable upstream Toasty or Topcoat API. |

`upstream` is the label that marks an upstream-tracking issue. The issue body
carries the status; a maintainer updates the label only if the issue stops being
an upstream gap.

## Triage roles

One role at a time. A maintainer sets it, and replaces it as the issue moves.

| Label | Meaning |
| --- | --- |
| `needs-triage` | Maintainer needs to evaluate this issue. |
| `needs-info` | Waiting on the reporter for more information. |
| `ready-for-agent` | Fully specified, ready for an AFK agent. |

## Applied by a maintainer

| Label | Meaning |
| --- | --- |
| `accessibility` | Barrier affecting people with disabilities. |
| `documentation` | Improvements or additions to documentation. |
| `duplicate` | This issue or pull request already exists. |
| `good first issue` | Good for newcomers. |
| `help wanted` | Extra attention is needed. |
| `invalid` | This does not seem right. |
| `question` | Further information is requested. |
| `wontfix` | This will not be worked on. |
