#!/bin/bash
# Graded comparison protocol: N repeats, crates measured in separate
# processes, first-crate alternates per repeat (cancels cold-start order tax).
# Usage: benchmark/compare.sh --repeats 5 --crates "loole flume" -- <bench args>
#   Optional env: TASKSET="0-7" prefixes runs with taskset -c; OUTDIR override.
set -u
REPEATS=5; CRATES="loole flume"; OUTDIR=""; TASKSET="${TASKSET:-}"
BENCH_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --repeats) REPEATS="$2"; shift 2;;
    --crates) CRATES="$2"; shift 2;;
    --outdir) OUTDIR="$2"; shift 2;;
    --) shift; BENCH_ARGS=("$@"); break;;
    *) echo "usage: compare.sh --repeats N --crates \"a b\" [--outdir D] -- [bench args]" >&2; exit 2;;
  esac
done
TS=$(date -u +%Y-%m-%d-%H-%M)
OUTDIR="${OUTDIR:-target/bench-runs/$TS}"
mkdir -p "$OUTDIR"
i=1
order="$CRATES"
while [ "$i" -le "$REPEATS" ]; do
  # shellcheck disable=SC2206
  crates_arr=($order)
  for crate in "${crates_arr[@]}"; do
    if [ -n "$TASKSET" ]; then prefix="taskset -c $TASKSET"; else prefix=""; fi
    # shellcheck disable=SC2086
    $prefix cargo bench -p benchmark --bench criterion_benchmarks -- --crates "$crate" "${BENCH_ARGS[@]}" || exit $?
    dest="$OUTDIR/repeat-$i/$crate"
    mkdir -p "$dest"
    python3 benchmark/summarize.py --snapshot --criterion-dir target/criterion --out "$dest" --cell-filter "${BENCH_CELL_FILTER:-spsc}" --repeat "$i" || exit $?
  done
  # alternate first-crate next repeat
  order="$(echo "$order" | tr ' ' '\n' | tac | tr '\n' ' ')"
  i=$((i + 1))
done
python3 benchmark/summarize.py --summarize "$OUTDIR"
