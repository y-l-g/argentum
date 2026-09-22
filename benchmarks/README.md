# Benchmarks — Phase 2 (Relations & polish)

Server-rendering performance harness for Argentum (Phase 2).

The harness measures the Argentum admin list (Topcoat-based) for the
Phase-2 workload, following the methodology of
`tokio-rs/topcoat/benchmarks/` (loopback HTTP/1.1 document requests,
`oha` load generator). The hand-written **Axum + Maud** and **Leptos**
apps are compile-only smoke (stubs, not comparable) since GH #159 — they
render no 50-row workload, so no cross-framework comparison exists.

Phase-2 workload: **list with 50 rows, 2 includes (`author` + `comments`),
tenancy set, `can_view_any` enforced**, measured on the real list path
(`TableState::from_cx` → `Table::load` over the tenant-scoped
`scoped_query` with the declared `.paginate(50)` → `render_with_state` →
HTML). The raw query-only
figure is kept as a labeled diagnostic alongside it. The budget is
**< 40 ms p50** on SQLite/Postgres local (TTFB dominated by the
slowest `defer` region's skeleton, not the query — `README.md:8`).

Layout:

```
benchmarks/
  argentum/    Argentum/Topcoat app under test (Phase-2 workload, --bench flag)
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

# The numbers are UNGATED (GH #171): the harness measures the real list path
# and prints the <40ms p50 budget for reference, but never PASS/FAILs on it —
# numbers are collected first, the p50 gate follows in a follow-up. The
# process exits nonzero only on harness errors (connect/load/render failure).
# CI's bench-check job compiles the harness with --locked and enforces the
# lockstep pins; it does not run the benchmark itself.

# Postgres leg (opt-in — no local Postgres assumed):
# ARGENTUM_BENCH_POSTGRES_URL=postgresql://toasty:toasty@localhost:5432/toasty \
#   cargo run --manifest-path benchmarks/argentum/Cargo.toml -- --bench
# The URL must name a disposable bench database (the leg resets it, pushes
# schema, and seeds under a fresh tenant each run). Without it, only the
# SQLite leg runs.

# Full bench incl. HTTP leg (requires `oha` for the HTTP leg; timings informational, ungated):
./benchmarks/scripts/bench.sh
# -> benchmarks/results/bench.json + results.md (argentum only; baselines smoke-only)

# Smoke + self-check (argentum 50 rows + baseline compiles):
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
  swaps only the table, not the shell.
* **Pagination** — cursor pagination (Toasty appends the PK tie-breaker internally).

Budget v1 (Phase 1): list (25 rows, 2 includes, 1 count) `< 40 ms p50`.
Budget v2 (Phase 2): list (50 rows, 2 includes) `< 40 ms p50` on the real
list path (from_cx → load → render_with_state → HTML), tenancy set and policy
enforced. GH #171 lands the honest bench UNGATED (numbers first, gate with
headroom in a follow-up): the harness prints the budget for reference and
never PASS/FAILs on it.

Results are written per run under `benchmarks/results/` (gitignored). CI's
bench-check job compiles the harness with `--locked` and verifies its
topcoat/toasty revs match the workspace lock; it does not run the benchmark.
The harness is intentionally detached so `cargo test --workspace` stays fast.

## Parity

Cross-framework HTML parity was dropped in GH #159 (stubs are
non-comparable). `verify_parity.sh` asserts the Argentum list renders the
50 rows (`Post 00..Post 49` with `Author` includes) and that both baseline
stubs still compile. See `benchmarks/scripts/verify_parity.sh`.
