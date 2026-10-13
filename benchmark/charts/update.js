"use strict";

// `npm run update -- <size> [payload]`: rebuild the benchmark-result JSONs
// and the latency-dist JSON from criterion output, then render the SVG
// charts from them. Only the `touched` mode has a chart set (`shared` is
// refused); the payload argument defaults to `touched` and exists so runs
// stay explicit.

const { execFileSync } = require("node:child_process");

const size = process.argv[2];
const payload = process.argv[3] || "touched";

for (const script of ["collect-criterion.js", "dist-charts.js", "main.js"]) {
  const args = ["--no-warnings", script];
  if (size) args.push(size);
  if (payload) args.push(payload);
  execFileSync(process.execPath, args, { stdio: "inherit", cwd: __dirname });
}
