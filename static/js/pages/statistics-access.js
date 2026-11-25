// The Users and access tab of the statistics page (san-1026): how many people
// and tokens hold what. Counts only; no user and no group is named.

/**
 * @typedef {{
 *   sign_in_mode: string, users: number, pending_approval: number,
 *   roles: StatisticsLabelCount[], groups: StatisticsLabelCount[],
 *   group_memberships: number, tokens: StatisticsLabelCount[], expired_tokens: number,
 * }} StatisticsAccess
 */

STATISTICS_RENDERERS.access = (/** @type {StatisticsAccess} */ data) => {
  /** @type {StatisticsLabelCount[]} */
  const overview = [
    { label: "Users", count: data.users },
    { label: "Waiting for approval", count: data.pending_approval },
    { label: "Groups", count: data.groups.reduce((n, g) => n + g.count, 0) },
    { label: "Group memberships", count: data.group_memberships },
    { label: "API tokens", count: data.tokens.reduce((n, t) => n + t.count, 0) },
    { label: "Expired API tokens", count: data.expired_tokens },
  ];
  /** @type {StatisticsPanel[]} */
  const panels = [
    {
      id: "access-overview",
      title: "Accounts and tokens",
      audience: "management and administrators",
      note: `sign-in mode: ${data.sign_in_mode}`,
      chart: statisticsBarChart(overview),
      chartHeight: 240,
      table: {
        columns: ["Kind", "Count"],
        rows: [["Sign-in mode", data.sign_in_mode], ...overview.map((o) => [o.label, o.count])],
      },
    },
    {
      id: "access-roles",
      title: "Users per role",
      audience: "administrators and auditors",
      note: "direct or through a group",
      chart: statisticsBarChart(data.roles),
      chartHeight: Math.max(180, 40 + data.roles.length * 26),
      table: { columns: ["Role", "Users"], rows: data.roles.map((r) => [r.label, r.count]) },
    },
    {
      id: "access-groups",
      title: "Groups by source",
      audience: "administrators",
      table: { columns: ["Source", "Groups"], rows: data.groups.map((g) => [g.label, g.count]) },
    },
    {
      id: "access-tokens",
      title: "API tokens by scope",
      audience: "administrators and auditors",
      note: "unexpired",
      chart: {
        tooltip: { trigger: "item" },
        legend: { bottom: 0 },
        series: [
          {
            type: "pie",
            radius: ["40%", "70%"],
            data: data.tokens.map((t) => ({ name: t.label, value: t.count })),
            label: { formatter: "{b}: {c}" },
          },
        ],
      },
      chartHeight: 240,
      table: { columns: ["Scope", "Tokens"], rows: data.tokens.map((t) => [t.label, t.count]) },
    },
  ];
  return panels;
};
