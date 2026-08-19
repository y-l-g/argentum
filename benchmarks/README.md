# Benchmarks

Server-rendering performance harness for Argentum (stub).

The real harness measures an Argentum admin app (Topcoat-based) against
hand-written **Axum + Maud** and **Leptos** implementations of the same page,
following the same methodology as `tokio-rs/topcoat/benchmarks/` (loopback
HTTP/1.1 document requests, `oha` load generator, parity checks via
`verify_parity.sh`).

This directory is a **stub**: each comparator is a minimal buildable "hello"
app so the wiring and measurement scripts exist before the real storefront is
built. The apps live in their own cargo workspaces on purpose:

- `argentum/` — the Argentum (Topcoat-based) app under test.
- `axum-maud/` — the hand-written Axum + Maud baseline.
- `leptos/` — the Leptos SSR comparator.

None of these are members of the root workspace (mirroring Topcoat, where the
detached comparators are excluded), so the harness never interferes with the
toolkit's own `cargo test` / `cargo clippy`.

## Layout

```
benchmarks/
  argentum/    Argentum/Topcoat app under test (stub)
  axum-maud/   Axum + Maud baseline (stub)
  leptos/      Leptos SSR comparator (stub)
  scripts/     bench.sh, verify_parity.sh (stubs)
  results/     benchmark output (gitignored)
```

## Running (stub)

```sh
# Build + run one comparator and hit it by hand:
cargo run -p storefront-argentum
```

The real `bench.sh` matrix lands with the storefront app (Phase 2+), at which
point this README grows to match Topcoat's benchmark docs.
