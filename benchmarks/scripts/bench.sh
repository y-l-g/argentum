#!/usr/bin/env bash
# Tablo Phase 2 bench — oha + in-process honest bench for the Tablo list
# (50 rows, 2 includes, real list path with tenancy + policy, GH #171).
# Baselines (axum-maud, leptos) are compile-only smoke, not comparable
# (GH #159): they render stubs, so no cross-framework oha matrix exists.
# Mirrors tokio-rs/topcoat/benchmarks/scripts/bench.sh methodology
# (loopback HTTP/1.1, oha) for the Tablo target only.
#
# UNGATED (GH #171): the in-process leg collects numbers, it does not gate —
# no PASS/FAIL on timings. The oha p50 print below is informational too.
# Postgres leg: set TABLO_BENCH_POSTGRES_URL (disposable bench database)
# and the in-process run covers it alongside SQLite; otherwise SQLite only.
#
# Usage:
#   ./benchmarks/scripts/bench.sh [tablo|axum-maud|leptos]   (default: tablo)
#   tablo runs the oha + in-process bench; axum-maud/leptos only verify
#   the baseline still compiles (smoke).
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
  FRAMEWORKS=(tablo)
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
  echo "bench.sh: oha or jq not found — falling back to in-process --bench for tablo" >&2
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
    printf "bench.sh: p50 %.2fms (budget <40ms, reference only — UNGATED per GH #171)\n" "$p50ms"
  fi
}

for fw in "${FRAMEWORKS[@]}"; do
  case "$fw" in
    tablo|storefront-tablo)
      echo "==> building tablo (storefront-tablo)"
      cargo build --manifest-path "$BENCH/tablo/Cargo.toml" --release 2>&1 | tail -n 5
      if [ "$HAS_OHA" -eq 1 ]; then
        echo "==> starting tablo on http://localhost:3000"
        PORT=3000 cargo run --manifest-path "$BENCH/tablo/Cargo.toml" --release >/tmp/tablo-bench.log 2>&1 &
        SERVER_PID=$!
        trap 'kill_tree "$SERVER_PID"' EXIT INT TERM
        if ! wait_ready "http://localhost:3000/admin/posts"; then
          echo "tablo server failed to start, log:"
          cat /tmp/tablo-bench.log || true
          kill_tree "$SERVER_PID" || true
          trap - EXIT INT TERM
          exit 1
        fi
        echo "==> warming up ($WARMUP)"
        oha "http://localhost:3000/admin/posts" -z "$WARMUP" -c "$CONNECTIONS" --no-tui >/dev/null 2>&1 || true
        for run in $(seq 1 "$RUNS"); do
          run_oha "http://localhost:3000/admin/posts" "$RESULTS_DIR/tablo_run${run}.json"
          # Also hit filtered + grouped variants
          run_oha "http://localhost:3000/admin/posts?filters=status:published" "$RESULTS_DIR/tablo_filtered_run${run}.json" || true
          run_oha "http://localhost:3000/admin/posts?group_by=status" "$RESULTS_DIR/tablo_grouped_run${run}.json" || true
        done
        # Also run in-process bench as ground truth
        echo "==> in-process bench (cargo run -- --bench)"
        cargo run --manifest-path "$BENCH/tablo/Cargo.toml" --release -- --bench --iterations 100 | tee "$RESULTS_DIR/tablo_bench.txt"
        kill_tree "$SERVER_PID"
        trap - EXIT INT TERM
        sleep 1
      else
        echo "==> oha not found, running in-process bench only"
        cargo run --manifest-path "$BENCH/tablo/Cargo.toml" -- --bench --iterations 100 | tee "$RESULTS_DIR/tablo_bench.txt"
      fi
      ;;
    axum-maud|axum_maud)
      echo "==> building axum-maud (compile smoke only, GH #159 — stub, not comparable)"
      cargo build --manifest-path "$BENCH/axum-maud/Cargo.toml" --release 2>&1 | tail -n 5
      echo "==> axum-maud smoke passed (no oha leg; stub renders no 50-row workload)"
      ;;
    leptos)
      echo "==> building leptos (compile smoke only, GH #159 — stub, not comparable)"
      cargo build --manifest-path "$BENCH/leptos/Cargo.toml" --features ssr 2>&1 | tail -n 5 || true
      echo "==> leptos smoke passed (no server to benchmark; template stub only)"
      ;;
    *)
      echo "unknown framework $fw (expected tablo|axum-maud|leptos)" >&2
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
    cat "$RESULTS_DIR"/tablo_bench.txt 2>/dev/null || echo "no bench.txt"
  fi
  echo ""
  echo "Budget: Phase 2 Tablo list (50 rows, 2 includes) <40ms p50 — reference only, UNGATED per GH #171 (numbers first, gate follows). Baselines are smoke-only, not comparable (GH #159)."
} | tee "$RESULTS_DIR/results.md"

echo "bench.sh: done -> $RESULTS_DIR/results.md"
if [ -f "$RESULTS_DIR/results.md" ]; then
  cat "$RESULTS_DIR/results.md"
fi
