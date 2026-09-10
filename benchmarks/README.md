# Benchmarks — Phase 2 (Relations & polish)

Server-rendering performance harness for Argentum (Phase 2).

The harness measures the Argentum admin list (Topcoat-based) against
hand-written **Axum + Maud** and **Leptos** baselines for the same page,
following the methodology of `tokio-rs/topcoat/benchmarks/` (loopback
HTTP/1.1 document requests, `oha` load generator, parity checks).

Phase-2 workload: **list with 50 rows, 2 includes (`author` + `comments`),
all `Policy`-checked, `Table` as `Boundary` with `#[memoize]`**, plus
`SelectFilter`/`TernaryFilter`/`DateFilter` composition and `group_by` in-memory.
The budget is **< 40 ms p50** on SQLite/Postgres local (TTFB dominated by the
slowest `defer` region's skeleton, not the query — `README.md:8`).

Layout:

```
benchmarks/
  argentum/    Argentum/Topcoat app under test (Phase-2 workload, --bench flag)
  axum-maud/   Axum + Maud baseline (stub, same 50-row HTML)
  leptos/      Leptos SSR comparator (stub)
  scripts/     bench.sh (oha matrix), verify_parity.sh
  results/     benchmark output (gitignored)
```

Detached workspaces (not members of the root workspace, mirroring Topcoat)
so the harness never interferes with `cargo test` / `clippy`.

## Running

```sh
# Bench the Argentum list (50 rows, 2 includes) without starting a server:
cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench --iterations 100

# The budget (<40ms p50) gates the COLD path (fresh Cx per iteration);
# FAIL exits nonzero so the budget can gate a local or on-demand run (GH #103).
# CI's bench-check job compiles the harness with --locked and enforces the
# lockstep pins; it does not run the benchmark itself.

# Full matrix vs baselines (requires `oha`):
./benchmarks/scripts/bench.sh
# -> benchmarks/results/bench.json + results.md

# Verify parity (all three render the same 50 rows):
./benchmarks/scripts/verify_parity.sh
```

`cargo run --manifest-path benchmarks/argentum/Cargo.toml` (no flag) still
starts the Topcoat server at `http://localhost:3000/` for manual inspection.

## What "fast" means (Phase 2)

* **Concurrent rendering** — sibling components and rows `try_join!` (no waterfalls).
* **Memoization** — `#[memoize]` on the loader (`Post::all().include(...).exec`)
  so streaming re-renders don't repeat I/O.
* **Preloading** — `include` for `author` + `comments` (3 operations, not 101).
* **Boundaries** — `Table` is a `Boundary` (`data-boundary="table"`); search/filter/page
  swaps only the grid, not the shell.
* **Pagination** — cursor pagination (Toasty appends the PK tie-breaker internally).

Budget v1 (Phase 1): list (25 rows, 2 includes, 1 count) `< 40 ms p50`.
Budget v2 (Phase 2): list (50 rows, 2 includes) `< 40 ms p50` on the cold path.

Results are written per run under `benchmarks/results/` (gitignored). CI's
bench-check job compiles the harness with `--locked` and verifies its
topcoat/toasty revs match the workspace lock; it does not run the benchmark.
The harness is intentionally detached so `cargo test --workspace` stays fast.

## Parity

`verify_parity.sh` fetches `http://localhost:3000/` from each comparator and
diffs the normalized HTML (ignoring whitespace and `data-boundary` ids) to
ensure the baselines render the same 50 rows as Argentum. See
`benchmarks/scripts/verify_parity.sh`.
