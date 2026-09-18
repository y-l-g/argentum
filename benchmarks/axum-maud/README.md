# Storefront: Axum + Maud

The hand-written Axum + Maud smoke stub under the benchmark harness (GH #159:
compile-only, not comparable). Serves a single static page as plain functions
returning `maud` templates, with no framework layer on top — it renders no
50-row workload.

```sh
cargo run
```

Detached from the Argentum workspace on purpose (see `../README.md`).
