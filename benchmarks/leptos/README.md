# Storefront: Leptos

The Leptos SSR smoke stub under the benchmark harness (GH #159:
compile-only, not comparable). A single page rendered server-side in islands
mode; it renders no 50-row workload.

Requires [cargo-leptos](https://github.com/leptos-rs/cargo-leptos) and the
`wasm32-unknown-unknown` target, exactly as in `tokio-rs/topcoat/benchmarks`:

```sh
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos --locked
cargo leptos build --release
```

Detached from the Argentum workspace on purpose (see `../README.md`).
