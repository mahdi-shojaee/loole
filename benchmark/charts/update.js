"use strict";

// `npm run update -- <size>`: rebuild the benchmark-result JSONs from
// criterion output, then render the SVG charts from them.

const { execFileSync } = require("node:child_process");

const size = process.argv[2];

for (const script of ["collect-criterion.js", "main.js"]) {
  const args = ["--no-warnings", script];
  if (size) args.push(size);
  execFileSync(process.execPath, args, { stdio: "inherit", cwd: __dirname });
}
