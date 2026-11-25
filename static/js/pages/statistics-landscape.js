// The Landscape tab of the statistics page (san-1022): what exists, how it is
// grouped, what is active and how the domains depend on each other.

/**
 * @typedef {{ name: string, members: string[] }} StatisticsGroup
 * @typedef {{ label: string, count: number }} StatisticsLabelCount
 * @typedef {{
 *   producers: string[], consumers: string[], products: StatisticsGroup[],
 *   domains: StatisticsGroup[], tags: StatisticsGroup[],
 *   peripheral_resources: StatisticsGroup[], external_producers: string[],
 *   active_in_range: string[], on_main: string[],
 *   api_types: StatisticsLabelCount[],
 *   flows: { from: string, to: string, pairs: number }[],
 *   capped_months: string[],
 * }} StatisticsLandscape
 */

/**
 * The note of a panel built from the audit log when a month was capped.
 * @param {string[]} capped
 */
function statisticsCappedNote(capped) {
  return capped.length > 0 ? `undercounted in ${capped.join(", ")} (too many entries)` : "";
}

/**
 * A horizontal bar chart of labelled counts.
 * @param {StatisticsLabelCount[]} items
 */
function statisticsBarChart(items) {
  return {
    grid: { left: 140, right: 30, top: 10, bottom: 30 },
    xAxis: { type: "value", minInterval: 1 },
    yAxis: { type: "category", data: items.map((i) => i.label), inverse: true },
    tooltip: { trigger: "axis" },
    series: [
      { type: "bar", data: items.map((i) => i.count), label: { show: true, position: "right" } },
    ],
  };
}

STATISTICS_RENDERERS.landscape = (/** @type {StatisticsLandscape} */ data) => {
  const systems = [...new Set([...data.producers, ...data.consumers])].sort();
  const active = new Set(data.active_in_range);
  const onMain = new Set(data.on_main);
  /** @type {StatisticsLabelCount[]} */
  const counts = [
    { label: "Producers", count: data.producers.length },
    { label: "Consumers", count: data.consumers.length },
    { label: "Products", count: data.products.length },
    { label: "Domains", count: data.domains.length },
    { label: "Backstage tags", count: data.tags.length },
    { label: "Peripheral Resources", count: data.peripheral_resources.length },
    { label: "External Producers", count: data.external_producers.length },
  ];
  /** @type {StatisticsLabelCount[]} */
  const activity = [
    { label: "Systems", count: systems.length },
    { label: "Active in range", count: systems.filter((s) => active.has(s)).length },
    { label: "On main", count: systems.filter((s) => onMain.has(s)).length },
  ];
  /** @type {(string | number)[][]} */
  const groupRows = [];
  for (const [kind, list] of /** @type {[string, StatisticsGroup[]][]} */ ([
    ["Product", data.products],
    ["Domain", data.domains],
    ["Backstage tag", data.tags],
    ["Peripheral Resource", data.peripheral_resources],
  ])) {
    for (const g of list) groupRows.push([kind, g.name, g.members.length, g.members.join(", ")]);
  }
  /** @type {Set<string>} */
  const nodeNames = new Set();
  for (const f of data.flows) {
    nodeNames.add(`${f.from} (consumers)`);
    nodeNames.add(`${f.to} (producers)`);
  }
  /** @type {StatisticsPanel[]} */
  const panels = [
    {
      id: "landscape-counts",
      title: "What exists",
      audience: "architects and management",
      chart: statisticsBarChart(counts),
      chartHeight: 280,
      table: { columns: ["Kind", "Count"], rows: counts.map((c) => [c.label, c.count]) },
    },
    {
      id: "landscape-active",
      title: "Active systems",
      audience: "management and maintainers",
      note: [
        "active = a call in the range; on main = an open trunk pin or trunk version",
        statisticsCappedNote(data.capped_months),
      ]
        .filter(Boolean)
        .join(" · "),
      chart: statisticsBarChart(activity),
      chartHeight: 200,
      table: {
        columns: ["System", "Active in range", "On main"],
        rows: systems.map((s) => [s, active.has(s) ? "yes" : "no", onMain.has(s) ? "yes" : "no"]),
      },
    },
    {
      id: "landscape-api-types",
      title: "API types",
      audience: "architects",
      note: "version lines (Producer × API type)",
      chart: {
        tooltip: { trigger: "item" },
        legend: { bottom: 0 },
        series: [
          {
            type: "pie",
            radius: ["40%", "70%"],
            data: data.api_types.map((t) => ({ name: t.label, value: t.count })),
            label: { formatter: "{b}: {c}" },
          },
        ],
      },
      chartHeight: 280,
      table: {
        columns: ["API type", "Version lines"],
        rows: data.api_types.map((t) => [t.label, t.count]),
      },
    },
    {
      id: "landscape-flows",
      title: "Dependencies between domains",
      audience: "architects",
      note: "distinct Consumer–Producer pairs, dev graph",
      wide: true,
      ...(data.flows.length > 0 && {
        chart: {
          tooltip: { trigger: "item" },
          series: [
            {
              type: "sankey",
              emphasis: { focus: "adjacency" },
              data: [...nodeNames].map((name) => ({ name })),
              links: data.flows.map((f) => ({
                source: `${f.from} (consumers)`,
                target: `${f.to} (producers)`,
                value: f.pairs,
              })),
            },
          ],
        },
      }),
      chartHeight: 360,
      table: {
        columns: ["Consumer domain", "Producer domain", "Pairs"],
        rows: data.flows.map((f) => [f.from, f.to, f.pairs]),
      },
    },
    {
      id: "landscape-groups",
      title: "Products, domains, tags and Peripheral Resources",
      audience: "architects",
      wide: true,
      table: { columns: ["Kind", "Name", "Members", "Who"], rows: groupRows },
    },
    {
      id: "landscape-external",
      title: "External Producers",
      audience: "architects",
      table: { columns: ["Producer"], rows: data.external_producers.map((p) => [p]) },
    },
  ];
  return panels;
};
