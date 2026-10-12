const { JSDOM } = require("jsdom");
const fs = require("node:fs");

const { window } = new JSDOM();
global.window = window;
global.document = window.document;
global.navigator = window.navigator;
global.navigator.userAgent = window.navigator.userAgent;
global.navigator.language = window.navigator.language;
const echarts = require("echarts");

const messageSize = process.argv[2] || "256";
const payload = process.argv[3] || "alloc";
// The `alloc` set keeps the historical payload-less filenames; `touched`
// gets a `-touched` suffix, mirroring collect-criterion.js.
const suffix = payload === "alloc" ? "" : `-${payload}`;
const titlePayload = payload === "alloc" ? "" : ` (${payload})`;

// Same palette as the branch-compare dashboard (build.mjs CRATE_COLORS) so
// crates are identifiable across both tools. loole keeps its filled default.
const CRATE_COLORS = {
  flume: "#5B8DEF",
  "async-channel": "#38BDF8",
  tokio: "#2FB58E",
  "kanal (std-mutex)": "#A78BFA",
  "crossbeam-channel": "#F472B6",
};
const CRATE_FADE_ALPHA = 0.5;

function hexToRgba(hex, alpha) {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return hex;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

// Crates render as filled bars at 50% opacity; loole stays solid-filled.
function crateItemStyle(name) {
  return {
    color: hexToRgba(CRATE_COLORS[name] || "#8892a0", CRATE_FADE_ALPHA),
  };
}

echarts.setPlatformAPI({
  createCanvas() {},
});

function renderChart(data, chanType, benchType, metric) {
  const def = data[chanType][benchType];
  const source = def.dataset.source;
  const header = source[0];
  const rows = source.slice(1);

  const keep = [];
  for (let j = 1; j < header.length; j++) {
    if (rows.some((row) => Number(row[j]) > 0)) keep.push(j);
  }

  const crateNames = keep.map((j) => header[j]);
  // A 0 cell means the crate does not support that capacity/scenario
  // (e.g. async-channel/tokio at bounded(0), crossbeam in async scenarios);
  // null draws no bar instead of a misleading zero-height one.
  const filteredSource = [
    ["scenario", ...crateNames],
    ...rows.map((row) => [row[0], ...keep.map((j) => (Number(row[j]) > 0 ? row[j] : null))]),
  ];
  const chart = echarts.init(null, null, {
    renderer: "svg",
    ssr: true,
    width: 800,
    height: 200,
  });
  const option = {
    animation: false,
    title: {
      text: `${def.title} - ${metric} - ${messageSize} bytes${titlePayload}`,
      textStyle: {
        fontSize: 14,
      },
      left: "4%",
      top: 8,
    },
    backgroundColor: "white",
    legend: {
      orient: "horizontal",
      left: "center",
      bottom: "5%",
      type: "scroll",
      data: crateNames.map((name) =>
        name === "loole" ? name : { name, itemStyle: crateItemStyle(name) }
      ),
    },
    grid: {
      left: "4%",
      right: "4%",
      bottom: "20%",
      // Room for the title row plus the y-axis name beneath it.
      top: 60,
      containLabel: true,
    },
    tooltip: {},
    dataset: { source: filteredSource },
    xAxis: { type: "category" },
    yAxis:
      metric === "latency"
        ? { name: "ns (lower is better)" }
        : { name: "messages/s (higher is better)" },
    series: crateNames.map((name) => ({
      type: "bar",
      name,
      itemStyle: name === "loole" ? undefined : crateItemStyle(name),
    })),
  };
  chart.setOption(option);
  const svgStr = chart.renderToSVGString();
  chart.dispose();
  fs.writeFileSync(`images/${chanType}-${benchType}-${metric}-${messageSize}${suffix}.svg`, svgStr);
}

function renderMetric(metric) {
  const jsonPath = `./benchmark-result-${metric}-${messageSize}${suffix}.json`;
  if (!fs.existsSync(jsonPath)) {
    console.log(`no ${metric} results for ${messageSize} bytes (${payload}); skipping ${metric} charts`);
    return;
  }
  const data = require(jsonPath);
  for (const chanType of Object.keys(data)) {
    for (const benchType of Object.keys(data[chanType])) {
      renderChart(data, chanType, benchType, metric);
    }
  }
}

renderMetric("throughput");
renderMetric("latency");
