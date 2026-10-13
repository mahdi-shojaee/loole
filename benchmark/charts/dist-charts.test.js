"use strict";

// Tests for dist-charts.js: per-run latency-distribution sidecars become the
// 12 unbounded percentile charts (see benchmark/src/dist_sidecar.rs).

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { describe, test } = require("node:test");

const dc = require("./dist-charts.js");

function writeSidecar(dir, name, cells, whenMs) {
  const runs = path.join(dir, "runs");
  fs.mkdirSync(runs, { recursive: true });
  const file = path.join(runs, name);
  fs.writeFileSync(file, JSON.stringify({ cells }));
  const t = new Date(whenMs);
  fs.utimesSync(file, t, t);
  return file;
}

function cell(topo, combo, cap, crate, p99) {
  return {
    topology: topo,
    senders: 1,
    receivers: 1,
    combo,
    cap,
    crate,
    messages_no: 100000,
    sample_every: 10,
    count: 10000,
    p10: 100,
    p50: 200,
    p95: 300,
    p99,
    p99_9: 500,
  };
}

describe("dist-charts", () => {
  test("collectDist merges sidecars newest-wins per cell", () => {
    const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "dist-"));
    writeSidecar(dir, "latency-dist-32-touched-1-111.json", [cell("spsc", "sync-sync", "unbounded", "loole", 400)], 1000);
    writeSidecar(dir, "latency-dist-32-touched-2-222.json", [cell("spsc", "sync-sync", "unbounded", "loole", 450)], 2000);

    const { cells } = dc.collectDist(dir, 32, "touched");
    assert.equal(cells.length, 1);
    assert.equal(cells[0].p99, 450);
  });

  test("collectDist skips empty cells and other sizes", () => {
    const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "dist-"));
    writeSidecar(
      dir,
      "latency-dist-32-touched-1-111.json",
      [{ ...cell("spsc", "sync-sync", "unbounded", "loole", 400), count: 0 }],
      1000
    );
    writeSidecar(dir, "latency-dist-64-touched-1-111.json", [cell("spsc", "sync-sync", "unbounded", "loole", 400)], 2000);

    const { cells } = dc.collectDist(dir, 32, "touched");
    assert.equal(cells.length, 0);
  });

  test("distCoverageProblem demands unbounded cells from every crate", () => {
    const full = [];
    for (const topo of ["spsc", "mpmc", "mpsc"]) {
      for (const combo of ["sync-sync", "async-async", "async-sync", "sync-async"]) {
        for (const crate of ["loole", "flume", "async-channel", "tokio", "kanal-std-mutex", "crossbeam-channel"]) {
          full.push(cell(topo, combo, "unbounded", crate, 400));
        }
      }
    }
    assert.equal(dc.distCoverageProblem(full), null);

    const missing = full.filter((c) => !(c.topology === "mpsc" && c.crate === "flume"));
    assert.match(dc.distCoverageProblem(missing), /mpsc/);
  });

  test("buildDistData shapes quantile rows per topology and combo", () => {
    const cells = [
      cell("spsc", "sync-sync", "unbounded", "loole", 400),
      cell("spsc", "sync-sync", "unbounded", "flume", 450),
      cell("spsc", "sync-sync", "b10", "loole", 999),
    ];
    const data = dc.buildDistData(cells);
    assert.deepEqual(Object.keys(data), ["spsc"]);
    const ds = data.spsc["sync-sync"].dataset.source;
    assert.deepEqual(ds[0], ["quantile", "loole", "flume"]);
    // Rows are the five quantiles in order; bounded cells never leak in.
    assert.deepEqual(
      ds.map((r) => r[0]),
      ["quantile", "p10", "p50", "p95", "p99", "p99.9"]
    );
    assert.deepEqual(ds[1], ["p10", 100, 100]);
    assert.deepEqual(ds[4], ["p99", 400, 450]);
  });
});
