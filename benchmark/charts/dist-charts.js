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

const cc = require("./collect-criterion.js");
const { formatTitle, COMBO_FLAVORS } = cc;

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

// Merges sidecars newest-wins per (topology, combo, cap, crate). Sidecars
// older than opts.minMtimeMs are stale (previous runs) and skipped, mirroring
// the estimate collector. Returns { cells, newestMtimeMs }.
function collectDist(criterionDir, size, payload, opts = {}) {
  const minMtime = opts.minMtimeMs ?? 0;
  const cells = new Map();
  let newestMtimeMs = 0;
  for (const file of listDistSidecars(criterionDir, size, payload)) {
    const mtime = fs.statSync(file).mtimeMs;
    if (mtime < minMtime) continue;
    let sidecar;
    try {
      sidecar = JSON.parse(fs.readFileSync(file, "utf8"));
    } catch {
      continue; // truncated write; the writer itself is atomic, be lenient
    }
    if (!sidecar || !Array.isArray(sidecar.cells)) continue;
    newestMtimeMs = Math.max(newestMtimeMs, mtime);
    for (const cell of sidecar.cells) {
      if (!cell || typeof cell.count !== "number" || cell.count === 0) continue;
      const key = [cell.topology, cell.combo, cell.cap, cell.crate].join("|");
      cells.set(key, cell);
    }
  }
  return { cells: [...cells.values()], newestMtimeMs };
}

// Cell key shared by dist cells (flat topology string) and estimate cells
// (topology object): the support matrix differs per crate (crossbeam is
// sync-sync only, tokio has no mpmc), so completeness is derived from what
// the estimates actually measured — never from a hardcoded crate list.
function distKey(cell) {
  const topo = typeof cell.topology === "string" ? cell.topology : cell.topology.name;
  return [topo, cell.combo, cell.cap, cell.crate].join("|");
}

// Why the merged set cannot render the charts, or null when complete: every
// unbounded estimate cell (the measurable matrix) needs a dist cell.
// `expected` are estimate cells (fresh per the run marker).
function distCoverageProblem(cells, expected) {
  if (expected.length === 0) {
    return "no estimate cells to cover";
  }
  const have = new Set(cells.map(distKey));
  const missing = expected.map(distKey).filter((k) => !have.has(k));
  if (missing.length === 0) return null;
  // Group "topo|combo|cap|crate" keys back into "topo/combo: missing crates".
  const byGroup = new Map();
  for (const key of missing) {
    const [topo, combo, cap, crate] = key.split("|");
    if (cap !== "unbounded") continue;
    const g = `${topo}/${combo}`;
    if (!byGroup.has(g)) byGroup.set(g, []);
    byGroup.get(g).push(crate);
  }
  const groups = [...byGroup.entries()].map(([g, cs]) => `${g}: missing ${cs.join(",")}`);
  return groups.length > 0 ? groups.join("; ") : null;
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
    data[topo] = {};
    for (const combo of COMBOS) {
      // Same title vocabulary as the estimate charts (flavors inline, no
      // payload suffix: touched is the default and only charted mode).
      const [sf, rf] = COMBO_FLAVORS[combo];
      data[topo][combo] = {
        title: formatTitle(sample.senders, sample.receivers, sf, rf),
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
  // Same full-matrix gate as the estimate charts: partial runs must not
  // rebuild committed charts.
  const marker = cc.readRunMarker(criterionDir, size, payload);
  if (!marker) {
    console.error(
      `no recorded ${payload} benchmark run for message size ${size} under ${criterionDir}\n` +
        "run the suite first:\n" +
        `  cargo bench -p benchmark --bench criterion_benchmarks -- -m ${size} --payload ${payload}`
    );
    process.exit(1);
  }
  const coverage = cc.markerCoverageProblem(marker);
  if (coverage) {
    console.error(
      `the latest size-${size} ${payload} run was a partial run (${coverage}); ` +
        "dist charts are only rebuilt from full-matrix runs. Re-run the full suite first:\n" +
        `  cargo bench -p benchmark --bench criterion_benchmarks -- -m ${size} --payload ${payload}`
    );
    process.exit(1);
  }
  // Completeness is derived from the fresh estimates — the ground truth of
  // what was measured — so crate-specific support gaps (crossbeam is
  // sync-sync only, tokio has no mpmc) never read as missing data.
  const estCells = cc.collect(criterionDir, size, { minMtimeMs: marker.started_at, payload });
  const expected = estCells.filter((c) => c.cap === "unbounded");
  const { cells } = collectDist(criterionDir, size, payload, { minMtimeMs: marker.started_at });
  if (cells.length === 0) {
    console.log(
      `no latency-dist sidecars for size ${size} (${payload}) under ${criterionDir}; skipping dist charts`
    );
    return;
  }
  const distCoverage = distCoverageProblem(cells, expected);
  if (distCoverage) {
    console.error(
      `latency-dist data for size ${size} (${payload}) is partial (${distCoverage}); ` +
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
