---
name: style
description: Always use this skill before writing or editing Rust code or documentation in the Argentum repository
---

# Code Style

## General

- Keep related code together: a struct is immediately followed by its inherent
  `impl` and then its trait impls, before the next struct in the file. Unit
  tests (`#[cfg(test)] mod tests`) go at the very bottom of the file.
- Free functions are allowed, but first consider whether a more idiomatic Rust
  grouping onto a struct exists.
- Unsafe code is not allowed: `unsafe_code` is denied at the workspace level.
- Avoid needless allocations; it is reasonable to refactor the code a bit to
  make it faster.

## New modules

Name a module's file after the module and place it alongside its directory
(`foo.rs` next to `foo/`), never `foo/mod.rs`. Existing `mod.rs` files are
grandfathered: do not rename them, and never touch
`crates/argentum-ui/src/components/primitives/`, which mirrors
`topcoat-ui-registry` verbatim for the xtask sync.

## Dependencies

Declare shared dependency versions in the top-level `Cargo.toml` under
`[workspace.dependencies]`. Crates pull them in with `workspace = true` and opt
into features there. Never blanket `cargo update`: `topcoat`/`toasty` track
`main` and bump deliberately (see `CONTRIBUTING.md`).

## Documentation

Item docs describe what something is and does and how to use it. Describe the
current state only; never reference previous iterations. Avoid mentioning
unrelated items, like "this is used by X to do Y". See the
[`prose`](../prose/SKILL.md) skill for the voice rules.

## Tests

Test discipline lives in [`TESTING.md`](../../../docs/dev/TESTING.md): every
test protects a specific behavior, expected values come from the intended
behavior rather than the implementation, and useless tests are deleted instead
of kept.
