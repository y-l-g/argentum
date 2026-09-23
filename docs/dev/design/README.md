# Design documents

Prospective, guide-level documents for cross-cutting public-API changes: the
contract reviewers accept before the implementation lands. Routine features and
bug fixes do not need one; a change that reshapes `Panel`, `Resource`, `Table`,
`Schema`, or the policy/tenancy seams does.

An ADR records a decision already taken. A design document proposes one. When
the implementation merges, the durable reasoning moves to an ADR and the design
document stays as the proposal record.

## Workflow

1. Open a feature-proposal issue describing the problem and the shape of the
   solution.
2. Land the design document under `docs/dev/design/` in its own PR. Reviewers
   debate the API on that PR; it contains no implementation.
3. Land the implementation as a follow-up PR once the design is accepted. The
   merged design document is the contract; review should not re-litigate
   decisions made there.

## Starting a new document

Copy [`_template.md`](_template.md) to `docs/dev/design/<feature-name>.md` and
fill it in. Keep the section order; if a section genuinely does not apply,
delete it and say why in one line rather than leaving it empty.
