"use strict";

// `npm run update -- <size> [payload]`: rebuild the benchmark-result JSONs
// and the latency-dist JSON from criterion output, then render the SVG
// charts from them. Only the `touched` mode has a chart set (`shared` is
// refused); the payload argument defaults to `touched` and exists so runs
// stay explicit.

const { execFileSync } = require("node:child_process");

const size = process.argv[2];
const payload = process.argv[3] || "touched";

function run(script) {
  const args = ["--no-warnings", script];
  if (size) args.push(size);
  if (payload) args.push(payload);
  execFileSync(process.execPath, args, { stdio: "inherit", cwd: __dirname });
}

run("collect-criterion.js");
// The dist charts are additive: their failure (e.g. no full-run sidecar
// yet) warns loudly but never blocks the estimate charts or their rendering.
try {
  run("dist-charts.js");
} catch (err) {
  console.error(`warning: dist charts skipped (${String(err.message || err).split("\n")[0]})`);
}
run("main.js");
