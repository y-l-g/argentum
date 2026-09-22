# Testing and benchmarks

How to test a panel, what the showcase covers, and the performance harness and its budget.

Unit render with `CxTestBuilder`. Cover each resource policy fn. Cover scoping in `query()`.

The showcase has integration tests per area (list, create, edit, delete, bulk, filters, tenancy,
uploads, export, auth) under `examples/showcase/tests/`.

```sh
cargo test -p showcase
node --test crates/argentum-ui/assets/selects.test.js   # the one JS unit test
cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench
./benchmarks/scripts/bench.sh
```

The shell scripts under `crates/argentum-ui/assets/` are plain browser scripts loaded through
`asset!`, so they have no build step and no test runner. `selects.js` splits its one pure decision
(`matchingOptions`) out and guards the rest behind `install()` so that single function can be
unit-tested on Node's built-in runner (GH #184); everything else in those files is covered by the
Rust-side markup assertions and manual checks.

Budget: 50-row list with 2 preloaded relations renders under 40ms p50 on local SQLite. **This is a
target, not a gate** (GH #171): the harness prints it "for reference only; UNGATED" and never
PASS/FAILs on it, and `benchmarks/results/` is gitignored, so no committed number exists on a fresh
checkout. The skeleton ships first, rows stream in after.

Setup details — the detached bench workspace, the `oha` methodology, and the compile-only
axum-maud/leptos stubs — are in `benchmarks/README.md`.
