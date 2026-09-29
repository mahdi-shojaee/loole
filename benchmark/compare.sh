#!/bin/bash
# Graded comparison protocol: N repeats, crates measured in separate
# processes, first-crate alternates per repeat (cancels cold-start order tax).
# Usage: benchmark/compare.sh --repeats 5 --crates "loole flume" -- [bench args]
#   Optional env: TASKSET="0-7" prefixes runs with taskset -c; OUTDIR override.
# Parallel smoke screening: --parallel "0,1 2,3" pins each crate to its own
#   CPU pair and runs one wave of pairs concurrently. Pairs map positionally
#   to --crates (crate i always runs on pair i mod nslots, every repeat); with
#   more crates than pairs the remainder run in a second wave. Each crate
#   builds and measures under its own target/parallel-build/<crate>/ directory
#   so concurrent runs never share cargo or criterion state (first use rebuilds
#   per crate). Results are screening only: crates share power, thermal
#   headroom, and memory bandwidth while running together, so never use them
#   for verdicts (the summary prints an explicit warning). --parallel conflicts
#   with TASKSET (pick one pinning method).
set -u
REPEATS=5; CRATES="loole flume"; OUTDIR=""; TASKSET="${TASKSET:-}"; PARALLEL=""; PARALLEL_GIVEN=0
BENCH_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --repeats) REPEATS="$2"; shift 2;;
    --crates) CRATES="$2"; shift 2;;
    --outdir) OUTDIR="$2"; shift 2;;
    --parallel) PARALLEL="$2"; PARALLEL_GIVEN=1; shift 2;;
    --) shift; BENCH_ARGS=("$@"); break;;
    *) echo "usage: compare.sh --repeats N --crates \"a b\" [--outdir D] [--parallel \"0,1 2,3\"] -- [bench args]" >&2; exit 2;;
  esac
done
TS=$(date -u +%Y-%m-%d-%H-%M)
# Absolute repo-rooted paths: the script may be invoked from any directory,
# and per-worker env dirs must resolve identically for cargo, the bench
# binary (which derives criterion paths from the exe location), and snapshots.
REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
OUTDIR="${OUTDIR:-$REPO_ROOT/target/bench-runs/$TS}"
SUMMARIZE="$REPO_ROOT/benchmark/summarize.py"
mkdir -p "$OUTDIR"
# shellcheck disable=SC2206
all_crates=($CRATES)
ncrates=${#all_crates[@]}
if [ "$ncrates" -eq 0 ]; then echo "error: --crates needs at least one crate" >&2; exit 2; fi

# Filesystem-safe per-crate build dir name (e.g. "kanal (std-mutex)" -> "kanal__std_mutex_").
sanitize() { printf '%s' "$1" | tr -c '[:alnum:]_' '_'; }

snapshot_one() { # $1=crate $2=criterion-dir $3=repeat
  local dest="$OUTDIR/repeat-$3/$1"
  mkdir -p "$dest"
  python3 "$SUMMARIZE" --snapshot --criterion-dir "$2" --out "$dest" --cell-filter "${BENCH_CELL_FILTER:-spsc}" --repeat "$3" || exit $?
}

if [ "$PARALLEL_GIVEN" -eq 0 ]; then
  i=1
  order="$CRATES"
  while [ "$i" -le "$REPEATS" ]; do
    # shellcheck disable=SC2206
    crates_arr=($order)
    for crate in "${crates_arr[@]}"; do
      if [ -n "$TASKSET" ]; then prefix="taskset -c $TASKSET"; else prefix=""; fi
      # shellcheck disable=SC2086
      $prefix cargo bench -p benchmark --bench criterion_benchmarks -- --crates "$crate" "${BENCH_ARGS[@]}" || exit $?
      snapshot_one "$crate" "$REPO_ROOT/target/criterion" "$i"
    done
    # alternate first-crate next repeat
    order="$(echo "$order" | tr ' ' '\n' | tac | tr '\n' ' ')"
    i=$((i + 1))
  done
else
  if [ -n "$TASKSET" ]; then echo "error: --parallel conflicts with TASKSET (pick one pinning method)" >&2; exit 2; fi
  # shellcheck disable=SC2206
  pairs=($PARALLEL)
  nslots=${#pairs[@]}
  if [ "$nslots" -eq 0 ]; then echo "error: --parallel needs at least one CPU pair" >&2; exit 2; fi
  {
    echo "WARNING: parallel smoke-only screening — crates share power/thermal/bandwidth;"
    echo "numbers are NOT valid for verdicts. Use sequential pinned repeats for decisions."
    echo "pairs: $PARALLEL"
    echo "crates: $CRATES"
    echo "started: $TS"
  } | tee "$OUTDIR/SMOKE_PARALLEL"
  i=1
  first_fail=0
  while [ "$i" -le "$REPEATS" ]; do
    idx=0
    while [ "$idx" -lt "$ncrates" ]; do
      pids=()
      # One wave: up to nslots crates concurrently, fixed crate->slot mapping.
      for s in $(seq 0 $((nslots - 1))); do
        ci=$((idx + s)); [ "$ci" -ge "$ncrates" ] && break
        crate=${all_crates[$ci]}
        pair=${pairs[$((ci % nslots))]}
        # Per-worker target AND criterion dirs: run_marker derives the
        # criterion dir 3 levels up from the bench exe, so distinct target
        # dirs alone still share one criterion dir — CRITERION_HOME (honored
        # by run_marker::criterion_dir) gives each worker its own. Charts
        # never see these dirs (each child is a partial run), by design.
        wroot="$REPO_ROOT/target/parallel-build/$(sanitize "$crate")"
        (
          taskset -c "$pair" env CARGO_TARGET_DIR="$wroot" CRITERION_HOME="$wroot/criterion" \
            cargo bench -p benchmark --bench criterion_benchmarks -- --crates "$crate" "${BENCH_ARGS[@]}" || exit $?
          snapshot_one "$crate" "$wroot/criterion" "$i"
        ) &
        pids+=($!)
      done
      for pid in "${pids[@]}"; do
        wait "$pid" || { rc=$?; if [ "$first_fail" -eq 0 ]; then first_fail=$rc; fi; }
      done
      if [ "$first_fail" -ne 0 ]; then echo "wave failed, aborting remaining waves (first failure: $first_fail)" >&2; exit "$first_fail"; fi
      idx=$((idx + nslots))
    done
    i=$((i + 1))
  done
fi
python3 "$SUMMARIZE" --summarize "$OUTDIR"
