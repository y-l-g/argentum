# Upstream notes

`topcoat` and `toasty` are git dependencies tracking their `main` branches. A
model's training data postdates neither branch: verify every upstream API from
the pinned checkout, never from memory.

## How

- Read the API from the local cargo git cache or a sibling checkout
  (`../topcoat`, `../toasty` when present), at the rev `Cargo.lock` pins — not
  at their branch tip, which has moved on.
- Quote the source file and symbol that proves the signature, the behavior, or
  the absence you rely on.
- When the pinned rev changes, re-verify the claims that cited it.

For temporary live-editing against a local checkout, add an uncommitted
`[patch]` section redirecting to `../topcoat` / `../toasty`, as the pin-policy
comment in the root `Cargo.toml` describes.
