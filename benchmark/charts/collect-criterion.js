"use strict";

// Rebuilds benchmark-result-{throughput,latency}-<size>.json from criterion's
// output (target/criterion/**/new/estimates.json), in the exact schema the
// existing renderer (main.js) expects.
//
// Benchmark IDs are self-describing (see benchmark/src/ids.rs):
//   <metric>-<size>-n<count>-<payload>/<topology>-s<S>r<R>/<combo>/<cap>/<crate>
// criterion sanitizes '/' in IDs to '_' in directory names; none of our tokens
// contain '_', so parsing stays unambiguous. e.g.:
//   throughput-256-n100000-alloc_mpsc-s1000r1/sync-sync_b10_loole/new/estimates.json
//
// Selection rule per cell: the deepest workload (-n) wins; within one
// workload, the latest run wins. `rm -rf target/criterion` before publish runs
// keeps this trivial.

const fs = require("node:fs");
const path = require("node:path");

// --- Token tables (mirror benchmark/src/ids.rs; keep in sync) ----------------

const CRATE_LABELS = {
  loole: "loole",
  flume: "flume",
  "async-channel": "async-channel",
  tokio: "tokio",
  "kanal-std-mutex": "kanal (std-mutex)",
  "crossbeam-channel": "crossbeam-channel",
};
// Column order of the charts (canonical order of ALL_CRATES in benchmark.rs).
const CRATE_ORDER = [
  "loole",
  "flume",
  "async-channel",
  "tokio",
  "kanal-std-mutex",
  "crossbeam-channel",
];

// Row order of the bounded-capacity charts; the unbounded charts use the four
// combos as scenario rows. UNBOUNDED_ROW_ORDER matches the historical
// SCENARIO_NAMES order of the old runner.
const BOUNDED_CAPS = ["b0", "b1", "b10", "b50", "b100"];
const UNBOUNDED_ROW_ORDER = ["async-async", "sync-sync", "async-sync", "sync-async"];

// "async-sync" -> ["async", "sync"] (sender flavor, receiver flavor)
const COMBO_FLAVORS = {
  "sync-sync": ["sync", "sync"],
  "async-async": ["async", "async"],
  "async-sync": ["async", "sync"],
  "sync-async": ["sync", "async"],
};

// --- Parsing criterion's directory names -------------------------------------

const GROUP_RE = /^(throughput|latency)-(\d+)-n(\d+)-(alloc|shared|touched)_(spsc|mpsc|mpmc)-s(\d+)r(\d+)$/;
const FUNC_RE = /^(sync-sync|async-async|async-sync|sync-async)_(b\d+|unbounded)_(.+)$/;

function parseId(groupDir, funcDir) {
  const g = GROUP_RE.exec(groupDir);
  if (!g) return null;
  const f = FUNC_RE.exec(funcDir);
  if (!f) return null;
  return {
    metric: g[1],
    size: Number(g[2]),
    count: Number(g[3]),
    payload: g[4],
    topology: { name: g[5], senders: Number(g[6]), receivers: Number(g[7]) },
    combo: f[1],
    cap: f[2],
    crate: f[3],
  };
}

// --- Values -----------------------------------------------------------------

// criterion's estimates are nanoseconds per iteration. A throughput iteration
// transfers `count` messages; a latency iteration runs `count` round trips.
function cellValue(meta, meanNs) {
  return meta.metric === "throughput"
    ? Math.round((meta.count * 1e9) / meanNs)
    : Math.round(meanNs / meta.count);
}

// --- Run markers (freshness) --------------------------------------------------
//
// The bench binary overwrites <criterionDir>/runs/last-<size>-alloc.json at the
// start of every run (see benchmark/src/run_marker.rs). Charts are built only
// from estimates written after the marker's timestamp of the requested size,
// so charts always reflect one defined run and target/criterion never needs
// manual cleaning.

const RUNS_DIR = "runs";

// Returns the parsed marker { started_at, ... } or null when absent/invalid.
function readRunMarker(criterionDir, size) {
  try {
    const raw = fs.readFileSync(path.join(criterionDir, RUNS_DIR, `last-${size}-alloc.json`), "utf8");
    const marker = JSON.parse(raw);
    return typeof marker.started_at === "number" ? marker : null;
  } catch {
    return null;
  }
}

// --- Scanning ---------------------------------------------------------------

// Reads <criterionDir>/<group>/<func>/new/estimates.json for benches of
// `size`. Returns one cell per (metric, topology, combo, cap, crate) using
// the deepest workload, tie-broken by latest run. If opts.minMtimeMs is set,
// estimates older than that (ms epoch) are treated as stale and skipped.
function collect(criterionDir, size, opts = {}) {
  const minMtime = opts.minMtimeMs ?? 0;
  const cells = new Map();
  if (!fs.existsSync(criterionDir)) return [];
  for (const groupDir of fs.readdirSync(criterionDir)) {
    const groupPath = path.join(criterionDir, groupDir);
    if (!fs.statSync(groupPath).isDirectory()) continue;
    for (const funcDir of fs.readdirSync(groupPath)) {
      const estimatesFile = path.join(groupPath, funcDir, "new", "estimates.json");
      let mean;
      let mtime;
      try {
        mean = JSON.parse(fs.readFileSync(estimatesFile, "utf8")).mean.point_estimate;
        mtime = fs.statSync(estimatesFile).mtimeMs;
      } catch {
        continue; // not a benchmark dir
      }
      if (typeof mean !== "number") continue;
      if (mtime < minMtime) continue; // written before the latest run's start
      const meta = parseId(groupDir, funcDir);
      if (!meta || meta.size !== size) continue;
      if (meta.payload !== "alloc") continue; // charts render alloc mode only
      const key = [
        meta.metric,
        meta.topology.name,
        meta.topology.senders,
        meta.topology.receivers,
        meta.combo,
        meta.cap,
        meta.crate,
      ].join("|");
      const prev = cells.get(key);
      if (!prev || meta.count > prev.count || (meta.count === prev.count && mtime > prev.mtime)) {
        cells.set(key, { ...meta, value: cellValue(meta, mean), mtime });
      }
    }
  }
  return [...cells.values()];
}

// --- Assembling the chart JSON ----------------------------------------------

function formatTitle(senders, receivers, senderFlavor, receiverFlavor) {
  const s = senders !== 1 ? "s" : "";
  const r = receivers !== 1 ? "s" : "";
  return `${senders} ${senderFlavor} sender${s} / ${receivers} ${receiverFlavor} receiver${r}`;
}

function unboundedTitle(senders, receivers) {
  const s = senders !== 1 ? "s" : "";
  const r = receivers !== 1 ? "s" : "";
  return `unbounded: ${senders} sender${s} / ${receivers} receiver${r}`;
}

function capRowLabel(capToken) {
  return `bounded(${capToken.slice(1)})`;
}

// One dataset table: header row + one row per (cap|combo), value 0 for cells
// that were not run or are unsupported (the renderer draws 0 as an absent bar).
function boundedDataset(cells, crates, combo) {
  const header = ["capacity", ...crates.map((c) => CRATE_LABELS[c])];
  const rows = BOUNDED_CAPS.map((cap) => [
    capRowLabel(cap),
    ...crates.map((crate) => valueOf(cells, { combo, cap, crate })),
  ]);
  return [header, ...rows];
}

function unboundedDataset(cells, crates) {
  const header = ["scenario", ...crates.map((c) => CRATE_LABELS[c])];
  const rows = UNBOUNDED_ROW_ORDER.map((combo) => [
    combo,
    ...crates.map((crate) => valueOf(cells, { combo, cap: "unbounded", crate })),
  ]);
  return [header, ...rows];
}

function valueOf(cells, { combo, cap, crate }) {
  const cell = cells.find((c) => c.combo === combo && c.cap === cap && c.crate === crate);
  return cell ? cell.value : 0;
}

// Builds the full JSON object for one metric from its collected cells.
// `cells` may contain all topologies of the run; we slice by topology here.
function buildData(metric, cells) {
  const mine = cells.filter((c) => c.metric === metric);
  const topologyNames = metric === "latency" ? ["spsc"] : ["mpsc", "mpmc", "spsc"];
  const data = {};
  for (const name of topologyNames) {
    const topoCells = mine.filter((c) => c.topology.name === name);
    if (topoCells.length === 0) continue;
    // Titles take senders/receivers from the newest cell of this topology.
    const latest = topoCells.reduce((a, b) => ((b.mtime || 0) > (a.mtime || 0) ? b : a));
    const { senders, receivers } = latest.topology;
    const crates = CRATE_ORDER.filter((c) => topoCells.some((cell) => cell.crate === c));
    const datasets = {};
    for (const combo of Object.keys(COMBO_FLAVORS)) {
      const [sf, rf] = COMBO_FLAVORS[combo];
      datasets[combo] = {
        title: formatTitle(senders, receivers, sf, rf),
        dataset: { source: boundedDataset(topoCells, crates, combo) },
      };
    }
    datasets.unbounded = {
      title: unboundedTitle(senders, receivers),
      dataset: { source: unboundedDataset(topoCells, crates) },
    };
    data[name] = datasets;
  }
  return data;
}

// --- CLI --------------------------------------------------------------------

function main() {
  const size = Number(process.argv[2] || process.env.MESSAGE_SIZE || "256");
  const criterionDir =
    process.env.CRITERION_DIR ||
    process.env.CRITERION_HOME ||
    path.resolve(__dirname, "..", "..", "target", "criterion");
  const marker = readRunMarker(criterionDir, size);
  if (!marker) {
    console.error(
      `no recorded benchmark run for message size ${size} under ${criterionDir}\n` +
        "run the suite first:\n" +
        `  cargo bench -p benchmark --bench criterion_benchmarks -- -m ${size}`
    );
    process.exit(1);
  }
  const cells = collect(criterionDir, size, { minMtimeMs: marker.started_at });
  if (cells.length === 0) {
    console.error(
      `the latest size-${size} run (started ${new Date(marker.started_at).toISOString()}) ` +
        "produced no matching results — re-run the bench suite:\n" +
        `  cargo bench -p benchmark --bench criterion_benchmarks -- -m ${size}`
    );
    process.exit(1);
  }
  console.log(
    `using ${cells.length} cells from the run started ${new Date(marker.started_at).toISOString()}`
  );
  for (const metric of ["throughput", "latency"]) {
    if (!cells.some((c) => c.metric === metric)) {
      console.log(`no ${metric} results for ${size} bytes; not writing ${metric} JSON`);
      continue;
    }
    const data = buildData(metric, cells);
    const out = path.join(__dirname, `benchmark-result-${metric}-${size}.json`);
    fs.writeFileSync(out, JSON.stringify(data, null, 2) + "\n");
    console.log(`wrote ${path.basename(out)} (${cells.filter((c) => c.metric === metric).length} cells)`);
  }
}

if (require.main === module) {
  main();
}

module.exports = { parseId, cellValue, collect, readRunMarker, buildData, main };
