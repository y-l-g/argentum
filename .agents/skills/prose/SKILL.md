---
name: prose
description: Always use this skill before writing long-form markdown documentation for Tablo
---

# Prose

Write per [`PROSE.md`](../../../docs/dev/PROSE.md). It is the authoritative
source for voice, banned words, structure, and where each kind of writing lives.
The rules below are the ones agents miss most often.

## Placement

- Vocabulary and domain terms: `CONTEXT.md`.
- Decisions: `docs/adr/`.
- Prospective API designs: `docs/dev/design/`.
- Upstream API freshness: `docs/dev/upstream-notes.md`.
- User guide: `docs/guide/` (mdBook); `README.md` is the short entry point.
- Contributor specs (commits, prose, labels, testing): `docs/dev/`.
- Agent tracker notes: `docs/agents/`.

## Structure

Lead with a code sample where a sample answers the question. Match the file's
existing line wrapping: prose files in this repo wrap near column 100, and
commit subjects stay under 100 characters per `COMMITS.md`.

## General

- Write in plain English. No fancy sentence structure.
- Document the current state only; never reference previous iterations ("this
  used to be A but is now B"). History belongs in a commit message or an ADR.
