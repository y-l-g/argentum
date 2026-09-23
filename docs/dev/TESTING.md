# Testing

Rules for every test in this repo: Rust integration tests under
`examples/showcase/tests/`, unit tests in `#[cfg(test)]` modules, the xtask
contract tests, and the JavaScript suites under `crates/argentum-ui/assets/`.

## Rules

- Every test protects a specific behavior or catches a plausible bug. Before
  writing it, identify what incorrect behavior would make it fail.
- Do not write tests that only verify hardcoded values. Pin behavior, not copy:
  assert row counts, redirects, database state, and link targets rather than the
  exact wording of a message or button. A passing rename must not break the
  suite.
- Derive expected results from the intended behavior. Do not calculate them by
  repeating the implementation or calling the same code being tested.
- Keep tests sensitive to broken behavior and tolerant of implementation changes
  that preserve correct behavior. Prefer structural asserts (the retry link
  target, the absence of the action chrome) over literal asserts (the toast
  text, the button label).
- If a test is useless, delete it. A test that passes on nearly any page, or
  that pins today's rendering choice against the documented roadmap, proves
  nothing and fights the feature it anticipates.

## Where tests live

- `examples/showcase/tests/` — the integration suite: HTTP requests against the
  runnable admin, asserting status codes, redirects, rendered structure, and
  database state.
- `#[cfg(test)] mod tests` at the bottom of a source file — unit tests for pure
  decisions (escaping, state decoding, hook contracts).
- `crates/argentum-ui/assets/*.test.js` — the browser-asset suites, run with
  `node --test`. Each suite's header names the behavior it protects; DOM halves
  are covered by the integration suite instead.
- `xtask/tests/` — one contract per file (`asset_hooks.rs`, `registry_sync.rs`);
  edge cases live as unit tests in `xtask/src/lib.rs`.
