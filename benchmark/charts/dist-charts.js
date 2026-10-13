"use strict";

// Builds latency-dist-<size>.json (per-message queueing-delay percentiles)
// from the dist sidecars bench runs write to
// <criterionDir>/runs/latency-dist-<size>-<payload>-<unix_ms>-<pid>.json
// (see benchmark/src/dist_sidecar.rs), in the dataset-table schema the
// renderer (main.js) expects.
//
// One chart per (topology, combo), unbounded cells only: unbounded is where
// queueing tails grow unmasked, and bounded-cap tails are
// backpressure-dominated. Rows are the charted quantiles, columns are
// crates. Like the estimate converter, the newest sidecar wins per cell,
// so isolate-mode children (one sidecar per crate) merge correctly.

const fs = require("node:fs");
const path = require("node:path");

// Canonical crate columns (same order as the estimate charts).
const CRATE_ORDER = [
  "loole",
  "flume",
  "async-channel",
  "tokio",
  "kanal-std-mutex",
  "crossbeam-channel",
];
const CRATE_LABELS = {
  loole: "loole",
  flume: "flume",
  "async-channel": "async-channel",
  tokio: "tokio",
  "kanal-std-mutex": "kanal (std-mutex)",
  "crossbeam-channel": "crossbeam-channel",
};

// Charted quantiles with their sidecar keys, ascending.
const QUANTILES = [
  ["p10", "p10"],
  ["p50", "p50"],
  ["p95", "p95"],
  ["p99", "p99"],
  ["p99_9", "p99.9"],
];

const COMBOS = ["sync-sync", "async-async", "async-sync", "sync-async"];
const TOPOLOGIES = ["spsc", "mpsc", "mpmc"];

// All sidecars for one (size, payload), newest last.
function listDistSidecars(criterionDir, size, payload) {
  const runsDir = path.join(criterionDir, "runs");
  let files;
  try {
    files = fs.readdirSync(runsDir);
  } catch {
    return [];
  }
  const prefix = `latency-dist-${size}-${payload}-`;
  return files
    .filter((f) => f.startsWith(prefix) && f.endsWith(".json"))
    .map((f) => path.join(runsDir, f))
    .sort((a, b) => fs.statSync(a).mtimeMs - fs.statSync(b).mtimeMs);
}

// Merges sidecars newest-wins per (topology, combo, cap, crate). Returns
// { cells, newestMtimeMs }.
function collectDist(criterionDir, size, payload) {
  const cells = new Map();
  let newestMtimeMs = 0;
  for (const file of listDistSidecars(criterionDir, size, payload)) {
    let sidecar;
    try {
      sidecar = JSON.parse(fs.readFileSync(file, "utf8"));
    } catch {
      continue; // truncated write; the writer itself is atomic, be lenient
    }
    if (!sidecar || !Array.isArray(sidecar.cells)) continue;
    const mtime = fs.statSync(file).mtimeMs;
    newestMtimeMs = Math.max(newestMtimeMs, mtime);
    for (const cell of sidecar.cells) {
      if (!cell || typeof cell.count !== "number" || cell.count === 0) continue;
      const key = [cell.topology, cell.combo, cell.cap, cell.crate].join("|");
      cells.set(key, cell);
    }
  }
  return { cells: [...cells.values()], newestMtimeMs };
}

// Why the merged set cannot render the 12 charts, or null when complete:
// every (topology, combo) needs unbounded cells from every canonical crate.
function distCoverageProblem(cells) {
  const missing = [];
  for (const topo of TOPOLOGIES) {
    for (const combo of COMBOS) {
      const have = new Set(
        cells
          .filter((c) => c.topology === topo && c.combo === combo && c.cap === "unbounded")
          .map((c) => c.crate)
      );
      const want = CRATE_ORDER.filter((c) => !have.has(c));
      if (want.length > 0) missing.push(`${topo}/${combo}: missing ${want.join(",")}`);
    }
  }
  return missing.length > 0 ? missing.join("; ") : null;
}

// One dataset table per (topology, combo): header + one row per quantile.
function distDataset(topoCells, combo, crates) {
  const header = ["quantile", ...crates.map((c) => CRATE_LABELS[c])];
  const rows = QUANTILES.map(([key, label]) => [
    label,
    ...crates.map((crate) => {
      const cell = topoCells.find(
        (c) => c.combo === combo && c.cap === "unbounded" && c.crate === crate
      );
      return cell ? cell[key] : 0;
    }),
  ]);
  return [header, ...rows];
}

// Builds the full JSON object: {topo: {combo: {title, dataset}}}.
function buildDistData(cells) {
  const data = {};
  for (const topo of TOPOLOGIES) {
    const topoCells = cells.filter((c) => c.topology === topo && c.cap === "unbounded");
    if (topoCells.length === 0) continue;
    const crates = CRATE_ORDER.filter((c) => topoCells.some((cell) => cell.crate === c));
    const sample = topoCells[0];
    const scope = `${sample.senders} sender${sample.senders !== 1 ? "s" : ""} / ${
      sample.receivers
    } receiver${sample.receivers !== 1 ? "s" : ""}`;
    data[topo] = {};
    for (const combo of COMBOS) {
      data[topo][combo] = {
        title: `${scope} (${combo})`,
        dataset: { source: distDataset(topoCells, combo, crates) },
      };
    }
  }
  return data;
}

// --- CLI --------------------------------------------------------------------

function main() {
  const size = Number(process.argv[2] || process.env.MESSAGE_SIZE || "256");
  const payload = process.argv[3] || process.env.PAYLOAD || "touched";
  const criterionDir =
    process.env.CRITERION_DIR ||
    process.env.CRITERION_HOME ||
    path.resolve(__dirname, "..", "..", "target", "criterion");
  const { cells } = collectDist(criterionDir, size, payload);
  if (cells.length === 0) {
    console.log(
      `no latency-dist sidecars for size ${size} (${payload}) under ${criterionDir}; skipping dist charts`
    );
    return;
  }
  const coverage = distCoverageProblem(cells);
  if (coverage) {
    console.error(
      `latency-dist data for size ${size} (${payload}) is partial (${coverage}); ` +
        "dist charts are only rebuilt from full-matrix runs. Re-run the full suite first:\n" +
        `  cargo bench -p benchmark --bench criterion_benchmarks -- -m ${size} --payload ${payload}`
    );
    process.exit(1);
  }
  const data = buildDistData(cells);
  const out = path.join(__dirname, `latency-dist-${size}.json`);
  fs.writeFileSync(out, JSON.stringify(data, null, 2) + "\n");
  console.log(`wrote ${path.basename(out)} (${cells.length} cells)`);
}

if (require.main === module) {
  main();
}

module.exports = { listDistSidecars, collectDist, distCoverageProblem, buildDistData, main };
