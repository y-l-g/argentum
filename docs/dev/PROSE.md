# Prose

Rules for every human-readable text in this repo: documentation, the README,
ADRs, code comments, PR descriptions, issue bodies, and commit bodies.

## Rules

- State what things are and what they do.
- Use active voice and present tense: "the engine executes the query", not
  "the query is executed".
- Document current behavior only. Omit historical decisions, deprecated
  approaches, removed APIs, and planned work. A sentence explaining what the
  code used to do belongs in a commit message or an ADR, not in the source.
- Prefer concrete examples to description: show the call, the output, or the
  error.
- Cut fluff. Every sentence carries information.
- No buzzwords or business jargon ("leverage", "synergy", "stakeholders",
  "deliverables").
- No weasel words: "very", "really", "quite", "somewhat".
- No dramatic terms ("critical", "crucial", "vital") unless something actually
  breaks.
- No figurative metaphors — pick the literal word. Recurring offenders to avoid
  by name: "under the hood" (say what the code does), "out of the box" (say "by
  default"), "first-class" (say what is supported), "magic" (say what happens),
  "lights up" (say "enables"), "footgun" (name the failure), and "lands" or
  "ships" as verbs for code existing (say "is added", "exists", or "releases").

## Structure

Start with what the thing is, then why it exists, then what it does, then how
to use it. Lead with a code sample where a sample answers the question.

**Bad**: "This component is critical for ensuring optimal query performance."

**Good**: "This component combines multiple database round trips into one."

## Applied to code comments

A comment earns its place by explaining WHY: a non-obvious invariant, a
workaround for a named upstream bug, or a safety argument. A comment that
restates what the next line plainly does is noise. Prefer one precise sentence
to a paragraph, and do not narrate the refactor or the debugging session that
produced the code.

## Where each kind of writing lives

- Vocabulary and domain terms: `CONTEXT.md`
- Decisions: `docs/adr/`
- User guide: `docs/guide/` (mdBook); `README.md` is the short entry point
- Contributor specs — commits, prose, labels: `docs/dev/`
- Issue bodies: the templates in `.github/ISSUE_TEMPLATE/`
