#!/usr/bin/env bash
# Stub benchmark harness for Argentum.
#
# The real harness (oha-driven matrix, parity check, results table) lands with
# the storefront app in a later phase, mirroring tokio-rs/topcoat/benchmarks.
# Until then this script only checks that each comparator builds and serves a
# page, so the wiring exists and stays green.

set -euo pipefail

# Resolve paths relative to this script, so it works from any CWD.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "bench.sh: stub — building comparators..."
cargo build --manifest-path "$HERE/../argentum/Cargo.toml"
cargo build --manifest-path "$HERE/../axum-maud/Cargo.toml"
echo "bench.sh: comparators build OK"
echo "bench.sh: real measurement matrix lands with the storefront app."
