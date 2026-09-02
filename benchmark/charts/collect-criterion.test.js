"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const cc = require("./collect-criterion.js");

// ---------------------------------------------------------------------------
// parseId
// ---------------------------------------------------------------------------

test("parseId parses a full benchmark path", () => {
  const parsed = cc.parseId("throughput-256-n100000_mpsc-s1000r1", "sync-sync_b10_loole");
  assert.deepEqual(parsed, {
    metric: "throughput",
    size: 256,
    count: 100000,
    topology: { name: "mpsc", senders: 1000, receivers: 1 },
    combo: "sync-sync",
    cap: "b10",
    crate: "loole",
  });
});

test("parseId parses latency spsc unbounded kanal", () => {
  const parsed = cc.parseId("latency-32-n50000_spsc-s1r1", "async-sync_unbounded_kanal-std-mutex");
  assert.equal(parsed.metric, "latency");
  assert.equal(parsed.size, 32);
  assert.equal(parsed.count, 50000);
  assert.deepEqual(parsed.topology, { name: "spsc", senders: 1, receivers: 1 });
  assert.equal(parsed.combo, "async-sync");
  assert.equal(parsed.cap, "unbounded");
  assert.equal(parsed.crate, "kanal-std-mutex");
});

test("parseId rejects non-matching dirs (select group, report dirs)", () => {
  assert.equal(cc.parseId("select", "async_select_recv_buffer_0"), null);
  assert.equal(cc.parseId("report", "index.html"), null);
  assert.equal(cc.parseId("throughput-256-n100000_mpsc-s1000r1", "bogus-format"), null);
  assert.equal(cc.parseId("junk", "sync-sync_b10_loole"), null);
});

// ---------------------------------------------------------------------------
// cellValue
// ---------------------------------------------------------------------------

test("cellValue computes throughput as msgs/sec from mean ns/iteration", () => {
  // 2s for 100k messages -> 50k msg/s
  assert.equal(cc.cellValue({ metric: "throughput", count: 100000 }, 2e9), 50000);
});

test("cellValue computes latency as ns per round trip", () => {
  assert.equal(cc.cellValue({ metric: "latency", count: 2000 }, 7_831_093.9), 3916);
});

// ---------------------------------------------------------------------------
// Structure parity with the committed chart JSON files
// ---------------------------------------------------------------------------

// Normalizes a parsed JSON tree: every number becomes null so structure,
// strings and ordering can be compared independent of measured values.
function shape(node) {
  if (Array.isArray(node)) return node.map(shape);
  if (node !== null && typeof node === "object") {
    return Object.fromEntries(Object.entries(node).map(([k, v]) => [k, shape(v)]));
  }
  return typeof node === "number" ? null : node;
}

const CAPS = [0, 1, 10, 50, 100];
const COMBOS = ["sync-sync", "async-async", "async-sync", "sync-async"];
const CRATES = ["loole", "flume", "async-channel", "tokio", "kanal-std-mutex", "crossbeam-channel"];

// All cells of the historical full run: topologies like the committed JSONs
// (mpsc 1000/1, mpmc 1000/10, spsc 1/1), all caps incl. unbounded, all crates.
function fullCellSet() {
  const cells = [];
  const topoFor = { spsc: { name: "spsc", senders: 1, receivers: 1 }, mpsc: { name: "mpsc", senders: 1000, receivers: 1 }, mpmc: { name: "mpmc", senders: 1000, receivers: 10 } };
  for (const metric of ["throughput", "latency"]) {
    const topos = metric === "latency" ? { spsc: topoFor.spsc } : topoFor;
    for (const topology of Object.values(topos)) {
      for (const combo of COMBOS) {
        for (const capN of [...CAPS, null]) {
          for (const crate of CRATES) {
            cells.push({
              metric,
              topology,
              combo,
              cap: capN === null ? "unbounded" : `b${capN}`,
              crate,
              value: 1,
            });
          }
        }
      }
    }
  }
  return cells;
}

for (const metric of ["throughput", "latency"]) {
  test(`structure matches committed benchmark-result-${metric}-256.json`, () => {
    const committed = JSON.parse(
      fs.readFileSync(path.join(__dirname, `benchmark-result-${metric}-256.json`), "utf8")
    );
    const built = cc.buildData(metric, fullCellSet());
    assert.deepEqual(shape(built), shape(committed));
  });
}

test("cell values land in the right spot; crates keep canonical order", () => {
  const cells = [
    { metric: "throughput", topology: { name: "spsc", senders: 1, receivers: 1 }, combo: "sync-sync", cap: "b10", crate: "tokio", value: 111 },
    { metric: "throughput", topology: { name: "spsc", senders: 1, receivers: 1 }, combo: "sync-sync", cap: "b10", crate: "loole", value: 222 },
    { metric: "throughput", topology: { name: "spsc", senders: 1, receivers: 1 }, combo: "sync-sync", cap: "b10", crate: "kanal-std-mutex", value: 333 },
  ];
  const data = cc.buildData("throughput", cells);
  const dataset = data.spsc["sync-sync"].dataset.source;
  assert.deepEqual(dataset[0], ["capacity", "loole", "tokio", "kanal (std-mutex)"]);
  const row = dataset.find((r) => r[0] === "bounded(10)");
  assert.deepEqual(row, ["bounded(10)", 222, 111, 333]);
});

test("unbounded dataset uses scenario rows in the historical order", () => {
  const cells = CRATES.map((crate) => ({
    metric: "throughput",
    topology: { name: "mpmc", senders: 1000, receivers: 10 },
    combo: "async-sync",
    cap: "unbounded",
    crate,
    value: 7,
  }));
  const data = cc.buildData("throughput", cells);
  const rows = data.mpmc.unbounded.dataset.source;
  assert.deepEqual(rows[0][0], "scenario");
  assert.deepEqual(
    rows.slice(1).map((r) => r[0]),
    ["async-async", "sync-sync", "async-sync", "sync-async"]
  );
  assert.equal(data.mpmc.unbounded.title, "unbounded: 1000 senders / 10 receivers");
});

test("missing cells are zero", () => {
  const cells = [
    { metric: "latency", topology: { name: "spsc", senders: 1, receivers: 1 }, combo: "sync-sync", cap: "b0", crate: "loole", value: 5 },
  ];
  const data = cc.buildData("latency", cells);
  const dataset = data.spsc["sync-sync"].dataset.source;
  const header = dataset[0];
  const row = dataset.find((r) => r[0] === "bounded(0)");
  // every column absent from the input is zero-filled
  for (let j = 1; j < header.length; j++) {
    assert.equal(row[j], header[j] === "loole" ? 5 : 0);
  }
});

// ---------------------------------------------------------------------------
// collect(): reading a criterion directory tree end to end
// ---------------------------------------------------------------------------

test("collect reads new/estimates.json, prefers newest mtime per cell, skips non-bench dirs", () => {
  const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "crit-"));
  const write = (groupDir, funcDir, mean, whenMs) => {
    const d = path.join(dir, groupDir, funcDir, "new");
    fs.mkdirSync(d, { recursive: true });
    fs.writeFileSync(path.join(d, "estimates.json"), JSON.stringify({ mean: { point_estimate: mean } }));
    const t = new Date(whenMs);
    fs.utimesSync(path.join(d, "estimates.json"), t, t);
  };
  write("throughput-32-n1000_spsc-s1r1", "sync-sync_b0_loole", 1e6, 1000);
  write("throughput-32-n1000_spsc-s1r1", "sync-sync_b0_loole", 2e6, 2000); // newer wins
  write("latency-32-n1000_spsc-s1r1", "async-async_b1_flume", 4e6, 3000);
  write("select", "async_select_recv_buffer_0", 1e6, 4000); // ignored

  const cells = cc.collect(dir, 32);
  const tp = cells.find(
    (c) => c.metric === "throughput" && c.crate === "loole" && c.cap === "b0"
  );
  assert.equal(tp.value, Math.round((1000 * 1e9) / 2e6));
  const lat = cells.find((c) => c.metric === "latency");
  assert.equal(lat.value, Math.round(4e6 / 1000));
  assert(!cells.some((c) => c.combo === undefined && c.metric !== undefined && c.crate === undefined));
});

test("collect keeps only the deepest workload (-n) per metric+size", () => {
  const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "crit-"));
  const write = (groupDir, funcDir, mean, whenMs) => {
    const d = path.join(dir, groupDir, funcDir, "new");
    fs.mkdirSync(d, { recursive: true });
    fs.writeFileSync(path.join(d, "estimates.json"), JSON.stringify({ mean: { point_estimate: mean } }));
    const t = new Date(whenMs);
    fs.utimesSync(path.join(d, "estimates.json"), t, t);
  };
  // same cell, different workload counts; a *newer mtime* on the smaller run
  // must not win against the deeper run's selection
  write("throughput-32-n1000_spsc-s1r1", "sync-sync_b0_loole", 1e6, 5000);
  write("throughput-32-n100000_spsc-s1r1", "sync-sync_b0_loole", 2e9, 1000);

  const cells = cc.collect(dir, 32);
  const tp = cells.filter((c) => c.metric === "throughput");
  assert.equal(tp.length, 1);
  assert.equal(tp[0].count, 100000);
  assert.equal(tp[0].value, Math.round((100000 * 1e9) / 2e9));
});

// ---------------------------------------------------------------------------
// Run markers (freshness): runs/last-<size>.json
// ---------------------------------------------------------------------------

test("readRunMarker parses a marker file", () => {
  const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "crit-"));
  fs.mkdirSync(path.join(dir, "runs"), { recursive: true });
  fs.writeFileSync(
    path.join(dir, "runs", "last-32.json"),
    JSON.stringify({ started_at: 1_700_000_000_000, message_size: 32 }) + "\n"
  );
  const marker = cc.readRunMarker(dir, 32);
  assert.equal(marker.started_at, 1_700_000_000_000);
});

test("readRunMarker returns null when missing or invalid", () => {
  const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "crit-"));
  assert.equal(cc.readRunMarker(dir, 32), null);
  fs.mkdirSync(path.join(dir, "runs"), { recursive: true });
  fs.writeFileSync(path.join(dir, "runs", "last-32.json"), "not json");
  assert.equal(cc.readRunMarker(dir, 32), null);
  fs.writeFileSync(path.join(dir, "runs", "last-32.json"), JSON.stringify({ nope: 1 }));
  assert.equal(cc.readRunMarker(dir, 32), null);
});

test("collect with minMtimeMs excludes estimates older than the run start", () => {
  const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "crit-"));
  const write = (groupDir, funcDir, mean, whenMs) => {
    const d = path.join(dir, groupDir, funcDir, "new");
    fs.mkdirSync(d, { recursive: true });
    fs.writeFileSync(path.join(d, "estimates.json"), JSON.stringify({ mean: { point_estimate: mean } }));
    const t = new Date(whenMs);
    fs.utimesSync(path.join(d, "estimates.json"), t, t);
  };
  // fresh cell (written after cutoff) and stale leftover cell (older run)
  write("throughput-32-n1000_spsc-s1r1", "sync-sync_b0_loole", 1e6, 2000);
  write("throughput-32-n1000_spsc-s1r1", "sync-sync_b0_flume", 9e9, 500);

  const fresh = cc.collect(dir, 32, { minMtimeMs: 1000 });
  assert.equal(fresh.length, 1);
  assert.equal(fresh[0].crate, "loole");
  assert.equal(fresh[0].value, Math.round((1000 * 1e9) / 1e6));

  // without a cutoff both cells are collected
  const all = cc.collect(dir, 32);
  assert.equal(all.length, 2);
});
