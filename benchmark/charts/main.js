const { JSDOM } = require("jsdom");
const fs = require("node:fs");

const { window } = new JSDOM();
global.window = window;
global.document = window.document;
global.navigator = window.navigator;
global.navigator.userAgent = window.navigator.userAgent;
global.navigator.language = window.navigator.language;
const echarts = require("echarts");

const data = require("./benchmark-result.json");

echarts.setPlatformAPI({
  createCanvas() {},
});

function renderChart(data, chanType, benchType) {
  const barsNo = data[chanType][benchType].dataset.source[0].length - 1;
  const supportedCrates = data[chanType][benchType].dataset.source[0].filter(
    (_, index) =>
      index > 0 &&
      data[chanType][benchType].dataset.source
        .slice(1)
        .some((a) => a[index] > 0)
  );
  const chart = echarts.init(null, null, {
    renderer: "svg",
    ssr: true,
    animation: false,
    width: 700,
    height: 220,
  });
  const option = {
    title: {
      text: data[chanType][benchType].title,
      textStyle: {
        fontSize: 14,
      },
      left: "center",
      top: "5%",
      backgroundColor: "#ddd8",
      borderColor: "#ccc",
      borderWidth: 1,
      padding: [5, 20],
    },
    backgroundColor: "white",
    legend: {
      orient: "horizontal",
      left: "center",
      bottom: "5%",
      data: supportedCrates,
    },
    grid: {
      left: "4%",
      right: "4%",
      bottom: "20%",
      top: "20%",
      containLabel: true,
    },
    tooltip: {},
    dataset: data[chanType][benchType].dataset,
    xAxis: { type: "category" },
    yAxis: {},
    series: Array(barsNo).fill({ type: "bar" }),
  };
  chart.setOption(option);
  const svgStr = chart.renderToSVGString();
  chart.dispose();
  fs.writeFileSync(`images/${chanType}-${benchType}.svg`, svgStr);
}

for (const chanType of Object.keys(data)) {
  for (const benchType of Object.keys(data[chanType])) {
    renderChart(data, chanType, benchType);
  }
}
