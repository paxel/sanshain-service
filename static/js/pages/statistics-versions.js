// The Versions and releases tab of the statistics page (san-1024): versions
// created over time by stability, promotions, versions per Producer, retired
// families, Outdated and Snapshot-pinned Consumers, and deprecated endpoints
// still required.

/**
 * @typedef {{
 *   periods: StatisticsPeriods,
 *   created: StatisticsSeries[],
 *   promotions: number[],
 *   per_producer: { producer: string, ga: number, snapshot: number }[],
 *   retired: StatisticsLabelCount[],
 *   pins: { consumer: string, producer: string, api_type: string, pin: string, latest_ga: string | null, state: string }[],
 *   deprecated_required: { consumer: string, producer: string, version: string, method: string, path: string }[],
 *   capped_months: string[],
 * }} StatisticsVersions
 */

STATISTICS_RENDERERS.versions = (/** @type {StatisticsVersions} */ data) => {
  const per = data.periods.bucket === "week" ? "week starting" : "day";
  const outdated = data.pins.filter((p) => p.state === "outdated").length;
  const snapshotPinned = data.pins.length - outdated;
  /** @type {StatisticsPanel[]} */
  const panels = [
    {
      id: "versions-created",
      title: "Versions created, GA against snapshot",
      audience: "management and maintainers",
      note: `per ${per}, by the stability the version has now`,
      wide: true,
      chart: statisticsTimeChart(data.periods, data.created),
      table: {
        columns: [per, ...data.created.map((s) => s.label)],
        rows: data.periods.starts.map((start, i) => [
          start,
          ...data.created.map((s) => s.counts[i] ?? 0),
        ]),
      },
    },
    {
      id: "versions-promotions",
      title: "Promotions to GA",
      audience: "management",
      note: [`per ${per}`, statisticsCappedNote(data.capped_months)].filter(Boolean).join(" · "),
      chart: statisticsTimeChart(data.periods, [{ label: "Promotions", counts: data.promotions }]),
      table: {
        columns: [per, "Promotions"],
        rows: data.periods.starts.map((start, i) => [start, data.promotions[i] ?? 0]),
      },
    },
    {
      id: "versions-per-producer",
      title: "Versions per Producer",
      audience: "maintainers",
      chart: {
        grid: { left: 140, right: 20, top: 30, bottom: 30 },
        legend: { top: 0 },
        tooltip: { trigger: "axis" },
        xAxis: { type: "value", minInterval: 1 },
        yAxis: { type: "category", inverse: true, data: data.per_producer.map((p) => p.producer) },
        series: [
          { name: "GA", type: "bar", stack: "total", data: data.per_producer.map((p) => p.ga) },
          {
            name: "Snapshot",
            type: "bar",
            stack: "total",
            data: data.per_producer.map((p) => p.snapshot),
          },
        ],
      },
      chartHeight: Math.max(200, 60 + data.per_producer.length * 24),
      table: {
        columns: ["Producer", "GA", "Snapshot"],
        rows: data.per_producer.map((p) => [p.producer, p.ga, p.snapshot]),
      },
    },
    {
      id: "versions-pins",
      title: "Outdated and Snapshot-pinned Consumers",
      audience: "architects and maintainers",
      note: `${String(outdated)} outdated · ${String(snapshotPinned)} snapshot-pinned`,
      chart: statisticsBarChart([
        { label: "Outdated", count: outdated },
        { label: "Snapshot-pinned", count: snapshotPinned },
      ]),
      chartHeight: 160,
      table: {
        columns: ["Consumer", "Producer", "API type", "Pin", "Latest GA", "State"],
        rows: data.pins.map((p) => [
          p.consumer,
          p.producer,
          p.api_type,
          p.pin,
          p.latest_ga ?? "–",
          p.state,
        ]),
      },
    },
    {
      id: "versions-deprecated",
      title: "Deprecated endpoints still required",
      audience: "architects and maintainers",
      wide: true,
      table: {
        columns: ["Consumer", "Producer", "Version", "Method", "Path"],
        rows: data.deprecated_required.map((d) => [
          d.consumer,
          d.producer,
          d.version,
          d.method,
          d.path,
        ]),
      },
    },
    {
      id: "versions-retired",
      title: "Retired API families",
      audience: "architects",
      note: "in the range",
      table: {
        columns: ["Producer", "Retirements"],
        rows: data.retired.map((r) => [r.label, r.count]),
      },
    },
  ];
  return panels;
};
