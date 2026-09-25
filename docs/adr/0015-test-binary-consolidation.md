# Integration tests: one binary per crate, not per file

Date: 2026-09-19 — Status: accepted — Amended: none

## Decision

Cargo builds one test binary per file in `tests/`, and each one links the crates it uses. Tablo's
integration tests all pull the full stack (topcoat's server runtime, toasty, bundled sqlite), so the
per-file targets cost ~160-190 MB of artifacts each and `cargo test --workspace` spent most of its
link time producing the same dependency graph over and over.

Both `examples/showcase` and `crates/tablo-core` therefore set `autotests = false` and declare a
single `[[test]] name = "it"` target. `tests/it.rs` declares each former test file as a module, and
each module imports the shared fixture through `crate::common::…`, so the fixture is compiled once per
crate instead of once per file. `xtask/tests/` keeps its two files: 30 MB total is not worth choking
on, and those tests do not link the server stack.

## Consequences

- A file's tests are now its module's: `cargo test --test it admin::` instead of
  `cargo test --test admin`; `cargo test -p showcase` runs everything.
- Failure isolation is gone: a compile error in any module fails the whole target, and a panic can no
  longer be attributed by binary name — the test *path* (`admin::some_test`) stays unique, which is
  what CI reports.
- The modules share one process and its thread pool; nothing here mutates process-wide state (no
  `set_var`, no fixed ports, no shared files), which is what makes that safe. A future test that needs
  isolation gets a dedicated target rather than a return to the file-per-binary default.
- Option 2 (a shared fixture crate) stays available if `tests/common` ever needs to be shared across
  *crates* rather than within one; option 3 (`debug = 0` in the test profile) remains rejected on
  debuggability grounds, as recorded in GH #179.
