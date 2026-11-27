// The Activity tab of the statistics page (san-1023): calls over time and per
// owner, the busiest systems and the refused provides. Counts only; no person
// is named.

/**
 * @typedef {{ bucket: string, week_start: string, starts: string[] }} StatisticsPeriods
 * @typedef {{ label: string, counts: number[] }} StatisticsSeries
 * @typedef {{
 *   periods: StatisticsPeriods,
 *   calls: StatisticsSeries[],
 *   refusals: number[],
 *   owners: { owner: string, provides: number, requires: number, catalog_pushes: number, refusals: number }[],
 *   top_systems: { system: string, owner: string, calls: number }[],
 *   capped_months: string[],
 * }} StatisticsActivity
 */

const STATISTICS_WEEKDAYS = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
];

/**
 * The background bands of a daily chart (san-1035): every second week, counted
 * from the admin's first day of the week, as `[first day, last day]` pairs.
 * A weekly chart has none.
 * @param {StatisticsPeriods} periods
 * @returns {[string, string][]}
 */
function statisticsWeekBands(periods) {
  if (periods.bucket !== "day") return [];
  const first = STATISTICS_WEEKDAYS.indexOf(periods.week_start);
  /** @type {[string, string][]} */
  const bands = [];
  let week = 0;
  /** @type {string | null} */
  let bandStart = null;
  let previous = "";
  for (const [i, day] of periods.starts.entries()) {
    if (i > 0 && new Date(`${day}T00:00:00Z`).getUTCDay() === first) {
      if (bandStart !== null) bands.push([bandStart, previous]);
      bandStart = null;
      week += 1;
    }
    if (week % 2 === 1 && bandStart === null) bandStart = day;
    previous = day;
  }
  if (bandStart !== null) bands.push([bandStart, previous]);
  return bands;
}

/**
 * A stacked bar chart over the periods of the range; a daily one bands every
 * second week.
 * @param {StatisticsPeriods} periods
 * @param {StatisticsSeries[]} series
 */
function statisticsTimeChart(periods, series) {
  const bands = statisticsWeekBands(periods);
  return {
    grid: { left: 50, right: 20, top: 30, bottom: 50 },
    legend: { top: 0 },
    tooltip: { trigger: "axis" },
    xAxis: { type: "category", data: periods.starts },
    yAxis: { type: "value", minInterval: 1 },
    series: series.map((s, i) => ({
      name: s.label,
      type: "bar",
      stack: "total",
      data: s.counts,
      ...(i === 0 &&
        bands.length > 0 && {
          markArea: {
            silent: true,
            itemStyle: { color: "rgba(148, 163, 184, 0.15)" },
            data: bands.map(([from, to]) => [{ xAxis: from }, { xAxis: to }]),
          },
        }),
    })),
  };
}

STATISTICS_RENDERERS.set("activity", (/** @type {StatisticsActivity} */ data) => {
  const per = data.periods.bucket === "week" ? "week starting" : "day";
  const capped = statisticsCappedNote(data.capped_months);
  /** @type {StatisticsPanel[]} */
  const panels = [
    {
      id: "activity-calls",
      title: "Calls over time",
      audience: "management and maintainers",
      note: [`per ${per}`, capped].filter(Boolean).join(" · "),
      wide: true,
      chart: statisticsTimeChart(data.periods, data.calls),
      table: {
        columns: [per, ...data.calls.map((s) => s.label)],
        rows: data.periods.starts.map((start, i) => [
          start,
          ...data.calls.map((s) => s.counts[i] ?? 0),
        ]),
      },
    },
    {
      id: "activity-owners",
      title: "Activity per owner",
      audience: "management",
      note: "the system's Backstage owner; a person is never named",
      chart: {
        grid: { left: 140, right: 20, top: 30, bottom: 30 },
        legend: { top: 0 },
        tooltip: { trigger: "axis" },
        xAxis: { type: "value", minInterval: 1 },
        yAxis: { type: "category", inverse: true, data: data.owners.map((o) => o.owner) },
        series: /** @type {const} */ ([
          ["Provides", "provides"],
          ["Requires", "requires"],
          ["Catalog pushes", "catalog_pushes"],
        ]).map(([name, key]) => ({
          name,
          type: "bar",
          stack: "total",
          data: data.owners.map((o) => o[key]),
        })),
      },
      chartHeight: Math.max(200, 60 + data.owners.length * 28),
      table: {
        columns: ["Owner", "Provides", "Requires", "Catalog pushes", "Refused"],
        rows: data.owners.map((o) => [
          o.owner,
          o.provides,
          o.requires,
          o.catalog_pushes,
          o.refusals,
        ]),
      },
    },
    {
      id: "activity-top",
      title: "Busiest systems",
      audience: "maintainers",
      chart: statisticsBarChart(data.top_systems.map((s) => ({ label: s.system, count: s.calls }))),
      chartHeight: Math.max(200, 40 + data.top_systems.length * 24),
      table: {
        columns: ["System", "Owner", "Calls"],
        rows: data.top_systems.map((s) => [s.system, s.owner, s.calls]),
      },
    },
    {
      id: "activity-refusals",
      title: "Refused provides",
      audience: "maintainers",
      note: `per ${per}`,
      chart: statisticsTimeChart(data.periods, [{ label: "Refused", counts: data.refusals }]),
      table: {
        columns: [per, "Refused"],
        rows: data.periods.starts.map((start, i) => [start, data.refusals[i] ?? 0]),
      },
    },
  ];
  return panels;
});
