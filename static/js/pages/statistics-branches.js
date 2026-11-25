// The Tags and branches tab of the statistics page (san-1025): the
// sanshain-branches with their cut date and size, and the builds per stream.

/**
 * @typedef {{
 *   branches: { name: string, cut_at: string, as_of: string, source: string, members: number }[],
 *   periods: StatisticsPeriods,
 *   builds: StatisticsSeries[],
 *   per_tag: StatisticsLabelCount[],
 *   capped_months: string[],
 * }} StatisticsBranches
 */

STATISTICS_RENDERERS.branches = (/** @type {StatisticsBranches} */ data) => {
  const per = data.periods.bucket === "week" ? "week starting" : "day";
  /** @type {StatisticsPanel[]} */
  const panels = [
    {
      id: "branches-builds",
      title: "Builds per stream",
      audience: "management and maintainers",
      note: [`provides and requires per ${per}`, statisticsCappedNote(data.capped_months)]
        .filter(Boolean)
        .join(" · "),
      wide: true,
      chart: statisticsTimeChart(data.periods, data.builds),
      table: {
        columns: [per, ...data.builds.map((s) => s.label)],
        rows: data.periods.starts.map((start, i) => [
          start,
          ...data.builds.map((s) => s.counts[i] ?? 0),
        ]),
      },
    },
    {
      id: "branches-list",
      title: "Sanshain-branches",
      audience: "architects and management",
      chart: statisticsBarChart(data.branches.map((b) => ({ label: b.name, count: b.members }))),
      chartHeight: Math.max(160, 40 + data.branches.length * 24),
      table: {
        columns: ["Branch", "Cut", "As of", "Source", "Member versions"],
        rows: data.branches.map((b) => [
          b.name,
          b.cut_at.slice(0, 10),
          b.as_of.slice(0, 10),
          b.source,
          b.members,
        ]),
      },
    },
    {
      id: "branches-tags",
      title: "Builds per tag",
      audience: "maintainers",
      note: "in the range",
      table: { columns: ["Tag", "Builds"], rows: data.per_tag.map((t) => [t.label, t.count]) },
    },
  ];
  return panels;
};
