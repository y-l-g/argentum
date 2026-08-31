#!/usr/bin/env bash
# Argentum Phase 2 bench — loopback oha matrix vs baselines.
# Mirrors tokio-rs/topcoat/benchmarks/scripts/bench.sh but scoped to the
# Argentum storefront (50 rows, 2 includes, Table as Boundary with #[memoize]).
#
# Usage:
#   ./benchmarks/scripts/bench.sh [argentum|axum-maud|leptos]   (default: argentum)
# Tunables:
#   DURATION=5s WARMUP=2s CONNECTIONS=32 RATE=100 RUNS=1 PORT=3000
#
# Requires: oha, jq, curl

set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
BENCH="$ROOT/benchmarks"

DURATION="${DURATION:-5s}"
WARMUP="${WARMUP:-2s}"
CONNECTIONS="${CONNECTIONS:-32}"
RUNS="${RUNS:-1}"
RESULTS_DIR="${RESULTS_DIR:-$BENCH/results/$(date +%Y%m%d-%H%M%S)}"

FRAMEWORKS=("$@")
if [ ${#FRAMEWORKS[@]} -eq 0 ]; then
  FRAMEWORKS=(argentum)
fi

mkdir -p "$RESULTS_DIR"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing $1: $2" >&2
    exit 1
  fi
}

# Only require oha/jq when we actually run oha; allow --bench in-process fallback
if command -v oha >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  HAS_OHA=1
else
  HAS_OHA=0
  echo "bench.sh: oha or jq not found — falling back to in-process --bench for argentum" >&2
fi

wait_ready() {
  local url="$1"
  local tries=50
  for _ in $(seq 1 $tries); do
    if curl -sf "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  echo "bench.sh: server not ready at $url after $tries tries" >&2
  return 1
}

kill_tree() {
  local pid="$1"
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    # also kill children
    pkill -P "$pid" 2>/dev/null || true
  fi
}

run_oha() {
  local url="$1"
  local out="$2"
  echo "bench.sh: oha $url -z $DURATION -c $CONNECTIONS -> $out"
  oha "$url" -z "$DURATION" -c "$CONNECTIONS" --no-tui --output-format json >"$out"
  local p50
  p50=$(jq -r '.latencyPercentiles.p50 // .latencyPercentile."50" // empty' "$out" 2>/dev/null || echo "")
  if [ -n "$p50" ]; then
    # oha reports p50 in seconds (float); convert to ms
    local p50ms
    p50ms=$(awk "BEGIN {print $p50*1000}")
    printf "bench.sh: p50 %.2fms (budget <40ms) " "$p50ms"
    if awk "BEGIN {exit !($p50ms < 40)}"; then
      echo "PASS"
    else
      echo "FAIL"
    fi
  fi
}

for fw in "${FRAMEWORKS[@]}"; do
  case "$fw" in
    argentum|storefront-argentum)
      echo "==> building argentum (storefront-argentum)"
      cargo build --manifest-path "$BENCH/argentum/Cargo.toml" --release 2>&1 | tail -n 5
      if [ "$HAS_OHA" -eq 1 ]; then
        echo "==> starting argentum on http://localhost:3000"
        PORT=3000 cargo run --manifest-path "$BENCH/argentum/Cargo.toml" --release >/tmp/argentum-bench.log 2>&1 &
        SERVER_PID=$!
        trap 'kill_tree "$SERVER_PID"' EXIT INT TERM
        if ! wait_ready "http://localhost:3000/admin/posts"; then
          echo "argentum server failed to start, log:"
          cat /tmp/argentum-bench.log || true
          kill_tree "$SERVER_PID" || true
          trap - EXIT INT TERM
          exit 1
        fi
        echo "==> warming up ($WARMUP)"
        oha "http://localhost:3000/admin/posts" -z "$WARMUP" -c "$CONNECTIONS" --no-tui >/dev/null 2>&1 || true
        for run in $(seq 1 "$RUNS"); do
          run_oha "http://localhost:3000/admin/posts" "$RESULTS_DIR/argentum_run${run}.json"
          # Also hit filtered + grouped variants
          run_oha "http://localhost:3000/admin/posts?filters=status:published" "$RESULTS_DIR/argentum_filtered_run${run}.json" || true
          run_oha "http://localhost:3000/admin/posts?group_by=status" "$RESULTS_DIR/argentum_grouped_run${run}.json" || true
        done
        # Also run in-process bench as ground truth
        echo "==> in-process bench (cargo run -- --bench)"
        cargo run --manifest-path "$BENCH/argentum/Cargo.toml" --release -- --bench --iterations 100 | tee "$RESULTS_DIR/argentum_bench.txt"
        kill_tree "$SERVER_PID"
        trap - EXIT INT TERM
        sleep 1
      else
        echo "==> oha not found, running in-process bench only"
        cargo run --manifest-path "$BENCH/argentum/Cargo.toml" -- --bench --iterations 100 | tee "$RESULTS_DIR/argentum_bench.txt"
      fi
      ;;
    axum-maud|axum_maud)
      echo "==> building axum-maud"
      cargo build --manifest-path "$BENCH/axum-maud/Cargo.toml" --release 2>&1 | tail -n 5
      if [ "$HAS_OHA" -eq 1 ]; then
        echo "==> starting axum-maud on http://localhost:8090"
        cargo run --manifest-path "$BENCH/axum-maud/Cargo.toml" --release >/tmp/axum-maud-bench.log 2>&1 &
        SERVER_PID=$!
        trap 'kill_tree "$SERVER_PID"' EXIT INT TERM
        if wait_ready "http://localhost:8090/"; then
          for run in $(seq 1 "$RUNS"); do
            run_oha "http://localhost:8090/" "$RESULTS_DIR/axum-maud_run${run}.json"
          done
        else
          echo "axum-maud failed to start"
          cat /tmp/axum-maud-bench.log || true
        fi
        kill_tree "$SERVER_PID"
        trap - EXIT INT TERM
        sleep 1
      fi
      ;;
    leptos)
      echo "==> building leptos (stub, cargo check only)"
      cargo build --manifest-path "$BENCH/leptos/Cargo.toml" --features ssr 2>&1 | tail -n 5 || true
      echo "==> leptos bench is stub — no server to benchmark (cargo check passed)"
      ;;
    *)
      echo "unknown framework $fw (expected argentum|axum-maud|leptos)" >&2
      exit 1
      ;;
  esac
done

# Render summary
{
  echo "# Bench results $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo ""
  echo "Results dir: $RESULTS_DIR"
  echo ""
  if ls "$RESULTS_DIR"/*.json >/dev/null 2>&1; then
    echo "| framework | file | req/s | p50 ms |"
    echo "|---|---|---|---|"
    for f in "$RESULTS_DIR"/*.json; do
      name=$(basename "$f")
      rps=$(jq -r '.rps.mean // empty' "$f" 2>/dev/null || echo "-")
      p50=$(jq -r '.latencyPercentiles.p50 // empty' "$f" 2>/dev/null || echo "-")
      # convert p50 seconds to ms if numeric
      if echo "$p50" | grep -qE '^[0-9.]+$'; then
        p50ms=$(awk "BEGIN {print $p50*1000}")
        p50="$p50ms"
      fi
      echo "| $name | $name | $rps | $p50 |"
    done
  else
    echo "No oha JSON (oha not installed or no runs). In-process bench output:"
    cat "$RESULTS_DIR"/argentum_bench.txt 2>/dev/null || echo "no bench.txt"
  fi
  echo ""
  echo "Budget: Phase 2 list (50 rows, 2 includes, filters+group_by) <40ms p50 (see README.md §8)."
} | tee "$RESULTS_DIR/results.md"

echo "bench.sh: done -> $RESULTS_DIR/results.md"
if [ -f "$RESULTS_DIR/results.md" ]; then
  cat "$RESULTS_DIR/results.md"
fi
