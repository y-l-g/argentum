# Integration tests: one binary per crate, not per file

Date: 2026-09-19 — Status: accepted — Supersedes: none

## Context

Cargo builds one test binary per file in `tests/`, and every one of them links
the crates it uses. Argentum's integration tests all pull the full stack —
topcoat's server runtime, toasty, bundled sqlite — so the showcase's fourteen
files produced fourteen executables of ~160-190 MB each, and
`cargo test --workspace` spent most of its link time producing the same
dependency graph over and over. Measured on the workspace before this change:

| Group | Binaries | Artifacts |
| --- | --- | --- |
| `examples/showcase/tests/` | 14 | 2218 MB |
| `crates/argentum-core/tests/` | 3 | 241 MB |
| `xtask/tests/` | 2 | 30 MB |

The build-side fix that landed first (`chore(build): stop forcing jobs=1 and
serialized codegen`) removed the artificial serialization; this ADR addresses
the target count itself. GH #179 laid out three mutually exclusive options:
consolidate into one `tests/it.rs` per crate, extract a shared fixture crate,
or drop test-profile debug info.

## Decision

**Consolidate.** `tests/it.rs` in `examples/showcase` and `argentum-core`
declares each former test file as a module:

```rust
mod common;
mod admin;
mod bulk_check;
// …
```

Each test file dropped its own `mod common;` and imports through
`crate::common::…`, so the shared fixture is compiled once per crate instead of
once per file. `xtask/tests/` keeps its two files: 30 MB total is not worth
choking on, and those tests do not link the server stack.

Measured after:

| Group | Binaries | Artifacts |
| --- | --- | --- |
| showcase | 1 | 245 MB |
| core | 1 | 109 MB |

About 2.1 GB of build artifacts and fifteen integration-test link steps fewer, with the whole
suite still green (117 showcase tests, 4 core integration tests).

## Consequences

- **Filtering changes.** A file's tests are now its module's:
  `cargo test --test it admin::` instead of `cargo test --test admin`.
  README §10 keeps `cargo test -p showcase`, which runs everything.
- **Failure isolation is gone.** A compile error in any module fails the whole
  target, and a panic in one test binary can no longer be attributed by binary
  name — the test *path* (`admin::some_test`) stays unique, which is what CI
  reports.
- **Parallelism changes.** Files that used to run as separate processes now
  share one process and its thread pool. Nothing here mutates process-wide state
  (no `set_var`, no fixed ports, no shared files), which is what made this safe;
  a future test that needs isolation should get a dedicated target rather than
  reintroduce the file-per-binary default.
- Option 2 (a `showcase-testkit` crate) stays available if `tests/common` ever
  needs to be shared across *crates* rather than within one.
- Option 3 (`debug = 0` in the test profile) remains rejected on debuggability
  grounds, as recorded in GH #179.
