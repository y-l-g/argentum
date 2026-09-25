# Benchmarks

Server-rendering performance harness for Argentum, following the methodology of
`tokio-rs/topcoat/benchmarks/` (loopback HTTP/1.1 document requests, `oha` load
generator). The hand-written **Axum + Maud** and **Leptos** apps are compile-only smoke
stubs, not comparable (GH #159): they render no 50-row workload.

Workload: **list with 50 rows, 2 includes (`author` + `comments`), tenancy set,
`can_view_any` enforced**, measured on the real list path (`TableState::from_cx` →
`Table::load` over the tenant-scoped `scoped_query` with the declared `.paginate(50)` →
`render_with_state` → HTML). The raw query-only figure is kept as a labeled diagnostic
alongside it. Budget: **< 40 ms p50** on local SQLite, with an opt-in Postgres leg
(see below). The numbers are UNGATED
(GH #171): the harness prints the budget for reference and never PASS/FAILs on it.

Layout:

```
benchmarks/
  argentum/    Argentum/Topcoat app under test (50-row workload, --bench flag)
  axum-maud/   Axum + Maud smoke stub (compiles; renders no 50-row workload)
  leptos/      Leptos SSR smoke stub (compiles; renders no 50-row workload)
  scripts/     bench.sh (argentum oha + in-process bench; baselines smoke-only), verify_parity.sh
  results/     benchmark output (gitignored)
```

Detached workspaces (not members of the root workspace, mirroring Topcoat)
so the harness never interferes with `cargo test` / `clippy`.

## Running

```sh
# Bench the Argentum list (50 rows, 2 includes) without starting a server:
cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench --iterations 100
# The process exits nonzero only on harness errors (connect/load/render failure).

# Postgres leg (opt-in — no local Postgres assumed):
# ARGENTUM_BENCH_POSTGRES_URL=postgresql://toasty:toasty@localhost:5432/toasty \
#   cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench
# The URL must name a disposable bench database (the leg resets it, pushes
# schema, and seeds under a fresh tenant each run). Without it, only the
# SQLite leg runs.

# Full bench incl. HTTP leg (requires `oha` for the HTTP leg; timings informational, ungated):
./benchmarks/scripts/bench.sh
# -> benchmarks/results/<timestamp>/results.md + oha JSON (argentum only; baselines smoke-only)

# Smoke + self-check (argentum 50 rows + baseline compiles):
./benchmarks/scripts/verify_parity.sh
```

`cargo run --manifest-path benchmarks/argentum/Cargo.toml` (no flag) still
starts the Topcoat server at `http://localhost:3000/` for manual inspection.

## What "fast" means

* **Preloading** — `include` for `author` + `comments` (3 operations, not 101).
* **Boundaries** — `Table` is a `Boundary` (`data-boundary="table"`); search/filter/page
  swaps only the table, not the shell.
* **Pagination** — cursor pagination (Toasty appends the PK tie-breaker internally).

Results are written per run under `benchmarks/results/` (gitignored). CI's
bench-check job compiles the harness with `--locked` and verifies its
topcoat/toasty revs match the workspace lock; it does not run the benchmark.

## Parity

Cross-framework HTML parity was dropped in GH #159 (stubs are
non-comparable). `verify_parity.sh` asserts the Argentum list renders the
50 rows (`Post 00..Post 49` with `Author` includes) and that both baseline
stubs still compile. See `benchmarks/scripts/verify_parity.sh`.
