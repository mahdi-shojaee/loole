#!/usr/bin/env python3
"""Winner-table summarizer for the graded comparison protocol (harness-v2).

Reads criterion ``estimates.json`` files (schema
``{"mean": {"point_estimate": <ns>}, ...}``) and turns them into
msgs/s winner tables.

Modes:
  --self-test
      Fixture check with zero dependencies (see ``run_self_test``).
  --cell <substring> [--criterion-dir DIR]
      One-shot table over ``DIR`` for every ``*/new/estimates.json`` whose
      full ID path contains ``substring``. Throughput cells report msgs/s as
      ``count * 1e9 / mean_ns`` with ``count`` parsed from the ``-n<count>``
      ID segment. Rows print ``crate, msgs_per_s`` sorted descending.
  --snapshot --out DEST [--criterion-dir DIR] [--cell-filter SUB]
             [--repeat N]
      Copy every matching ``*/new/estimates.json`` into ``DEST`` preserving
      relative paths, then write ``DEST/summary.json`` shaped
      ``{"repeat": N, "cells": {base_cell: {crate: msgs_per_s}}}`` where
      ``base_cell`` is the full ID minus the trailing crate segment so the
      two crates of one cell compare directly.
  --summarize <run-dir>
      Read every ``summary.json`` under ``<run-dir>/repeat-*/`` (both the
      flat ``repeat-N/summary.json`` layout and the per-crate
      ``repeat-N/<crate>/summary.json`` layout ``compare.sh`` writes),
      print per-cell winner tables plus a ``wins: loole=X flume=Y`` tally
      across repeats.

Only the standard library is used.
"""

import argparse
import contextlib
import io
import json
import os
import re
import shutil
import sys
import tempfile

COUNT_RE = re.compile(r"-n(\d+)")

# Canonical column order (benchmark::benchmark::ALL_CRATES); the wins tally
# prints known crates in this order so the graded line reads
# ``wins: loole=X flume=Y``. Unknown names sort alphabetically after these.
CANONICAL_CRATES = [
    "loole",
    "flume",
    "async-channel",
    "tokio",
    "kanal-std-mutex",
    "kanal (std-mutex)",
    "kanal",
    "crossbeam-channel",
]

# Longest-first so "kanal-std-mutex" wins over a bare "kanal" prefix and the
# old-style ``sync-sync_b10_kanal-std-mutex`` ID splits correctly (a naive
# rsplit('_', 1) would yield crate "mutex").
KNOWN_CRATE_TOKENS = sorted(
    set(CANONICAL_CRATES),
    key=len,
    reverse=True,
)


def crate_order_key(name):
    """Sort key for the wins tally: canonical crates first, then alpha."""
    if name in CANONICAL_CRATES:
        return (0, CANONICAL_CRATES.index(name))
    return (1, name)


def parse_count(cell_id):
    """Message count from the ``-n<count>`` ID segment, or None."""
    m = COUNT_RE.search(cell_id)
    return int(m.group(1)) if m else None


def split_crate(cell_id):
    """Split a full criterion ID into (base_cell, crate).

    New-style IDs carry the crate as the last path segment
    (``.../async-async/b100/loole``); old-style IDs fuse it with an
    underscore (``.../async-async_b100_loole``). Both map to the same base
    so loole-vs-flume rows of one cell group together.
    """
    parts = cell_id.split("/")
    last = parts[-1]
    for tok in KNOWN_CRATE_TOKENS:
        if last == tok and len(parts) > 1:
            return ("/".join(parts[:-1]), tok_normalize(tok))
        suffix = "_" + tok
        if last.endswith(suffix):
            base_last = last[: -len(suffix)]
            return ("/".join(parts[:-1] + [base_last]), tok_normalize(tok))
    # Fallback for unknown future crates: old vs new heuristic.
    if "_" in last:
        head, _, tail = last.rpartition("_")
        return ("/".join(parts[:-1] + [head]), tail)
    if len(parts) > 1:
        return ("/".join(parts[:-1]), last)
    return (cell_id, cell_id)


def tok_normalize(tok):
    """Map the kanal display name to its path token."""
    if tok in ("kanal", "kanal (std-mutex)"):
        return "kanal-std-mutex"
    return tok


def crate_of(cell_id):
    """Crate token for one full estimate ID."""
    return split_crate(cell_id)[1]


def base_of(cell_id):
    """Comparison base: full ID minus the trailing crate segment."""
    return split_crate(cell_id)[0]


def msgs_per_s(cell_id, mean_ns):
    """Throughput for a cell: count*1e9/mean for ``-n<count>`` IDs.

    Every current ID (throughput and latency) carries ``-n<count>``
    (messages-per-iteration / round-trips), so one formula covers both;
    IDs without a count fall back to plain per-second rate.
    """
    if not mean_ns or mean_ns <= 0:
        return 0.0
    count = parse_count(cell_id)
    if count is None:
        return 1e9 / mean_ns
    return count * 1e9 / mean_ns


def read_mean(estimates_path):
    """mean.point_estimate (ns) from one estimates.json."""
    with open(estimates_path, encoding="utf-8") as fh:
        data = json.load(fh)
    return float(data["mean"]["point_estimate"])


def iter_estimate_files(criterion_dir, cell_filter):
    """Yield (cell_id, estimates_path) for matches under criterion_dir."""
    matches = []
    for root, _dirs, files in os.walk(criterion_dir):
        if os.path.basename(root) != "new":
            continue
        if "estimates.json" not in files:
            continue
        est = os.path.join(root, "estimates.json")
        rel = os.path.relpath(est, criterion_dir)
        parts = rel.split(os.sep)
        if len(parts) < 3 or parts[-1] != "estimates.json" or parts[-2] != "new":
            continue
        cell_id = "/".join(parts[:-2])
        if cell_filter and cell_filter not in cell_id:
            continue
        matches.append((cell_id, est))
    matches.sort()
    return matches


def collect_cells(criterion_dir, cell_filter):
    """Collect (cell_id, crate, base, mean_ns, msgs) rows, or raise."""
    rows = []
    for cell_id, est in iter_estimate_files(criterion_dir, cell_filter):
        mean_ns = read_mean(est)
        crate = crate_of(cell_id)
        rows.append(
            {
                "cell": cell_id,
                "crate": crate,
                "base": base_of(cell_id),
                "mean_ns": mean_ns,
                "msgs": msgs_per_s(cell_id, mean_ns),
            }
        )
    return rows


def winner_of(crates_to_msgs):
    """(winner_crate, margin_pct) over {crate: msgs}; margin vs runner-up."""
    ranked = sorted(crates_to_msgs.items(), key=lambda kv: kv[1], reverse=True)
    winner, top = ranked[0]
    if len(ranked) < 2 or ranked[1][1] <= 0:
        return (winner, 0.0)
    margin = (top - ranked[1][1]) / ranked[1][1] * 100.0
    return (winner, margin)


def format_cell_table(rows):
    """Human-readable table with one row per crate, sorted by msgs desc."""
    ordered = sorted(rows, key=lambda r: r["msgs"], reverse=True)
    lines = ["crate                msgs_per_s  cell"]
    for r in ordered:
        lines.append(f"{r['crate']:20s} {r['msgs']:10.0f}  {r['cell']}")
    return "\n".join(lines)


def do_cell(substring, criterion_dir):
    rows = collect_cells(criterion_dir, substring)
    if not rows:
        print("no matching cells")
        return 1
    print(format_cell_table(rows))
    return 0


def do_snapshot(criterion_dir, out, cell_filter, repeat):
    matches = iter_estimate_files(criterion_dir, cell_filter)
    cells = {}
    copied = 0
    for cell_id, est in matches:
        rel = os.path.relpath(est, criterion_dir)
        dest = os.path.join(out, rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.copy2(est, dest)
        copied += 1
        mean_ns = read_mean(est)
        base, crate = split_crate(cell_id)
        cells.setdefault(base, {})[crate] = msgs_per_s(cell_id, mean_ns)
    if repeat is None:
        m = re.search(r"repeat-(\d+)", out)
        repeat = int(m.group(1)) if m else 0
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "summary.json"), "w", encoding="utf-8") as fh:
        json.dump({"repeat": repeat, "cells": cells}, fh, indent=2, sort_keys=True)
    print(f"snapshot: {copied} files -> {out} (repeat {repeat})")
    return 0


def find_summaries(run_dir):
    """All summary.json paths under run_dir, oldest-first (mtime, then path).

    Newest-wins merging in ``do_summarize`` relies on this order: within one
    repeat, ``compare.sh`` snapshots per crate sequentially, so the later
    (fresher, both-crates-current) file must override the earlier one whose
    other-crate rows may still be stale from the previous repeat.
    """
    found = []
    for root, _dirs, files in os.walk(run_dir):
        if "summary.json" in files:
            found.append(os.path.join(root, "summary.json"))
    found.sort(key=lambda p: (os.path.getmtime(p), p))
    return found


def smoke_warning(run_dir):
    """Warning header when this run used parallel smoke screening, else None.

    `compare.sh --parallel` writes a SMOKE_PARALLEL marker (pair assignment)
    into the run dir; numbers produced under shared power/thermal/bandwidth
    are screening-only, so every summary must say so up front.
    """
    marker = os.path.join(run_dir, "SMOKE_PARALLEL")
    if not os.path.isfile(marker):
        return None
    try:
        with open(marker, encoding="utf-8") as fh:
            detail = " ".join(line.strip() for line in fh if line.strip())
    except OSError:
        detail = ""
    base = (
        "WARNING: parallel smoke-only results (shared power/thermal/bandwidth)"
        " — not valid for verdicts."
    )
    return base + (" " + detail if detail else "")


def do_summarize(run_dir):
    warning = smoke_warning(run_dir)
    if warning is not None:
        print(warning)
    paths = find_summaries(run_dir)
    if not paths:
        print(f"no repeat summaries found under {run_dir}")
        return 1
    # per base -> per repeat -> per crate -> msgs (newest file wins ties).
    data = {}
    for path in paths:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
        repeat = doc.get("repeat")
        if repeat is None:
            m = re.search(r"repeat-(\d+)", path)
            repeat = int(m.group(1)) if m else path
        for base, per_crate in doc.get("cells", {}).items():
            data.setdefault(base, {}).setdefault(repeat, {}).update(per_crate)
    for base in sorted(data):
        print(f"cell: {base}")
        wins = {}
        means = {}
        for repeat in sorted(data[base]):
            per_crate = data[base][repeat]
            detail = " ".join(
                f"{c}={v:.0f}" for c, v in sorted(per_crate.items())
            )
            if len(per_crate) >= 2:
                winner, _ = winner_of(per_crate)
                wins[winner] = wins.get(winner, 0) + 1
                print(f"  repeat {repeat}: {detail} winner={winner}")
            else:
                print(f"  repeat {repeat}: {detail}")
            for c, v in per_crate.items():
                means.setdefault(c, []).append(v)
        avg = {c: sum(v) / len(v) for c, v in means.items()}
        if len(avg) >= 2:
            winner, margin = winner_of(avg)
            mean_str = " ".join(f"{c}={v:.0f}" for c, v in sorted(avg.items()))
            print(f"  mean: {mean_str} winner={winner} margin={margin:.1f}%")
        for c in means:
            wins.setdefault(c, 0)
        ordered_wins = sorted(wins.items(), key=lambda kv: crate_order_key(kv[0]))
        print("  " + "wins: " + " ".join(f"{c}={n}" for c, n in ordered_wins))
    return 0


def write_fixture_estimates(path, mean_ns):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump({"mean": {"point_estimate": float(mean_ns)}}, fh)


def run_self_test():
    """Brief's fixture: 800k vs 2M msgs/s must yield flume @ 150.0%."""
    tmp = tempfile.mkdtemp(prefix="summarize-self-test-")
    try:
        loole_cell = "throughput-16384-n20000-alloc/spsc-s1r1/async-async_b100_loole"
        flume_cell = "throughput-16384-n20000-alloc/spsc-s1r1/async-async_b100_flume"
        write_fixture_estimates(
            os.path.join(tmp, loole_cell, "new", "estimates.json"), 25_000_000
        )
        write_fixture_estimates(
            os.path.join(tmp, flume_cell, "new", "estimates.json"), 10_000_000
        )
        rows = collect_cells(tmp, "throughput-16384-n20000")
        assert len(rows) == 2, f"expected 2 fixture rows, got {len(rows)}"
        by_crate = {r["crate"]: r["msgs"] for r in rows}
        assert abs(by_crate["loole"] - 800_000) < 1e-6, by_crate
        assert abs(by_crate["flume"] - 2_000_000) < 1e-6, by_crate
        winner, margin = winner_of(by_crate)
        assert winner == "flume", f"winner={winner}"
        assert abs(margin - 150.0) < 1e-6, f"margin={margin}"
        table = format_cell_table(rows)
        assert "loole" in table and "flume" in table, f"table=\n{table}"
        # Snapshot + summarize round-trip over the same fixtures.
        snap = os.path.join(tmp, "snap")
        rc = do_snapshot(tmp, snap, "throughput-16384-n20000", 1)
        assert rc == 0, f"snapshot rc={rc}"
        run_dir = os.path.join(tmp, "runs")
        os.makedirs(os.path.join(run_dir, "repeat-1", "loole"))
        shutil.copytree(
            os.path.join(snap, "throughput-16384-n20000-alloc"),
            os.path.join(run_dir, "repeat-1", "loole", "throughput-16384-n20000-alloc"),
        )
        shutil.copy2(
            os.path.join(snap, "summary.json"),
            os.path.join(run_dir, "repeat-1", "loole", "summary.json"),
        )
        rc = do_summarize(run_dir)
        assert rc == 0, f"summarize rc={rc}"
        # SMOKE_PARALLEL marker forces a smoke-only warning header.
        with open(os.path.join(run_dir, "SMOKE_PARALLEL"), "w", encoding="utf-8") as fh:
            fh.write("pairs: 0,1 2,3\ncrates: loole flume\n")
        assert smoke_warning(run_dir) is not None
        assert smoke_warning(os.path.join(tmp, "no-such-dir")) is None
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = do_summarize(run_dir)
        assert rc == 0, f"summarize rc={rc}"
        assert "WARNING: parallel smoke-only results" in buf.getvalue(), buf.getvalue()
    except AssertionError as err:
        print(f"self-test: FAIL ({err})")
        return 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("self-test: PASS")
    return 0


def build_parser():
    p = argparse.ArgumentParser(description="Summarize criterion estimates.")
    p.add_argument("--self-test", action="store_true")
    p.add_argument("--cell", metavar="SUBSTRING", default=None)
    p.add_argument("--criterion-dir", default="target/criterion")
    p.add_argument("--snapshot", action="store_true")
    p.add_argument("--out", default=None)
    p.add_argument("--cell-filter", default="")
    p.add_argument("--repeat", type=int, default=None)
    p.add_argument("--summarize", metavar="RUN_DIR", default=None)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    if args.self_test:
        return run_self_test()
    if args.snapshot:
        if not args.out:
            print("error: --snapshot requires --out DEST", file=sys.stderr)
            return 2
        return do_snapshot(args.criterion_dir, args.out, args.cell_filter, args.repeat)
    if args.cell is not None:
        return do_cell(args.cell, args.criterion_dir)
    if args.summarize is not None:
        return do_summarize(args.summarize)
    build_parser().print_usage(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
