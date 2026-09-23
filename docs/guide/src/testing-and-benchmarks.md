# Testing and benchmarks

How to test a panel, what the showcase covers, and the performance harness and its budget.

Unit render with `CxTestBuilder`. Cover each resource policy fn. Cover scoping in `query()`.

The showcase has integration tests per area (list, create, edit, delete, bulk, filters, tenancy,
uploads, export, auth) under `examples/showcase/tests/`.

```sh
cargo test -p showcase
node --test crates/argentum-ui/assets/*.test.js   # the shell asset suites
cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench
./benchmarks/scripts/bench.sh
```

The shell scripts under `crates/argentum-ui/assets/` are plain browser scripts loaded through
`asset!`, so they have no build step and no runner of their own. Each carries a guarded
`module.exports` at the bottom so Node's built-in runner can reach it: `selects.test.js` covers the
combobox filter rule (GH #184), the hide-and-drop-`required` wiring (GH #236, GH #249) and the
single wiring pass (GH #237), and `bulk.test.js` covers the selection helpers and the disabled-box
skip (GH #235). The Rust-side markup assertions cover what the scripts leave alone.

Budget: 50-row list with 2 preloaded relations renders under 40ms p50 on local SQLite. **This is a
target, not a gate** (GH #171): the harness prints it "for reference only; UNGATED" and never
PASS/FAILs on it, and `benchmarks/results/` is gitignored, so no committed number exists on a fresh
checkout. The skeleton ships first, rows stream in after.

Setup details — the detached bench workspace, the `oha` methodology, and the compile-only
axum-maud/leptos stubs — are in `benchmarks/README.md`.
