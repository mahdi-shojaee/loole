"use strict";

// `npm run update -- <size> [payload]`: rebuild the benchmark-result JSONs
// from criterion output, then render the SVG charts from them. Payload
// defaults to `alloc` (historical filenames); `touched` writes the `-touched`
// suffixed set.

const { execFileSync } = require("node:child_process");

const size = process.argv[2];
const payload = process.argv[3];

for (const script of ["collect-criterion.js", "main.js"]) {
  const args = ["--no-warnings", script];
  if (size) args.push(size);
  if (payload) args.push(payload);
  execFileSync(process.execPath, args, { stdio: "inherit", cwd: __dirname });
}
