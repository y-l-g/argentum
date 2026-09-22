# Issue tracker

Issues live in GitHub at `y-l-g/argentum`. Use the `gh` CLI, which infers the
repo when run inside a clone.

## Commands

- Create: `gh issue create --title "..." --body "..."`. Use a heredoc for a
  multi-line body; the forms in `.github/ISSUE_TEMPLATE/` set the label.
- Read: `gh issue view <number> --comments`
- List: `gh issue list --state open --label upstream --json number,title,labels`
- Comment: `gh issue comment <number> --body "..."`
- Label: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- Close: `gh issue close <number> --comment "..."`

GitHub shares one number space between issues and pull requests, so a bare
`#123` can be either: try `gh pr view 123`, then `gh issue view 123`.

The label vocabulary is in [`docs/dev/LABELS.md`](../dev/LABELS.md).

## Upstream issues

An issue labeled `upstream` records a missing or unstable Toasty or Topcoat API
and the Argentum workaround it forces. File one with the Upstream gap form,
which fixes the five fields.

The body is the status. Edit the body when the upstream state changes; never
discuss status in a comment, so a reader learns the state from the body alone.
