// The small version strips at the top of the Producer and Consumer pages
// (san-1032). Producer: one bar per version, as high as the number of
// Consumers that pin it in the chosen scope (main by default, dev, or a
// sanshain-branch); a click names them. Consumer: one column per version of
// its require set (the `requires` section of its sanshain.yaml), with the Pins
// that arrived under it as chips.

/* exported renderProducerStrip, renderConsumerStrip */

/**
 * @param {string} serviceName
 * @param {HTMLElement} card an empty card the strip draws into
 */
async function renderProducerStrip(serviceName, card) {
  /** @type {{ name: string }[]} */
  let branches = [];
  try {
    branches = /** @type {{ name: string }[]} */ (await fetchJSON("/admin/branches")) ?? [];
  } catch {
    /* only main and dev then */
  }
  const options = [
    ["main", "main"],
    ["dev", "dev (all activity)"],
    ...branches.map((b) => [b.name, b.name]),
  ]
    .map(
      ([value, label]) =>
        `<option value="${escapeHtml(value ?? "")}">${escapeHtml(label ?? "")}</option>`,
    )
    .join("");
  card.innerHTML = `<div class="flex flex-wrap items-center justify-between gap-2 mb-1">
      <h3 class="font-semibold text-slate-800">Who pins which version</h3>
      <label class="text-xs text-slate-500">scope
        <select data-strip-scope class="ml-1 text-xs border border-slate-300 rounded-md px-1 py-0.5">${options}</select>
      </label>
    </div>
    <div data-strip-chart style="height: 140px"></div>
    <p data-strip-names class="text-xs text-slate-600 min-h-4"></p>`;
  const select = /** @type {HTMLSelectElement} */ (card.querySelector("[data-strip-scope]"));
  const chartEl = /** @type {HTMLElement} */ (card.querySelector("[data-strip-chart]"));
  const names = /** @type {HTMLElement} */ (card.querySelector("[data-strip-names]"));
  /** @type {echarts.ECharts | null} */
  let chart = null;
  const draw = async () => {
    /** @type {PinnedVersion[]} */
    let bars;
    try {
      bars = /** @type {PinnedVersion[]} */ (
        await fetchJSON(
          `/admin/producers/${encodeURIComponent(serviceName)}/pinned-by?scope=${encodeURIComponent(select.value)}`,
        )
      );
    } catch (err) {
      names.textContent = /** @type {Error} */ (err).message;
      return;
    }
    const types = new Set(bars.map((b) => b.api_type));
    const label = (/** @type {PinnedVersion} */ b) =>
      types.size > 1 ? `${b.api_type} ${b.version}` : b.version;
    chart?.dispose();
    chart = echarts.init(chartEl, null, { renderer: "svg" });
    chart.setOption({
      grid: { left: 30, right: 10, top: 10, bottom: 30 },
      xAxis: { type: "category", data: bars.map(label) },
      yAxis: { type: "value", minInterval: 1 },
      tooltip: {
        trigger: "item",
        formatter: (/** @type {{ dataIndex: number }} */ p) => {
          const b = bars[p.dataIndex];
          return b
            ? `${escapeHtml(label(b))}: ${escapeHtml(b.consumers.join(", ") || "nobody")}`
            : "";
        },
      },
      series: [
        {
          type: "bar",
          data: bars.map((b) => ({
            value: b.consumers.length,
            itemStyle: { color: b.stability === "ga" ? "#4f46e5" : "#94a3b8" },
          })),
        },
      ],
    });
    chart.on("click", (p) => {
      const b = bars[p.dataIndex];
      if (b) names.textContent = `${label(b)}: ${b.consumers.join(", ") || "nobody pins it"}`;
    });
    card.dataset.stripBars = String(bars.length);
    names.textContent = "";
  };
  select.addEventListener("change", () => void draw());
  await draw();
}

/**
 * @param {string} clientName
 * @param {HTMLElement} card an empty card the strip draws into
 */
async function renderConsumerStrip(clientName, card) {
  /** @type {RequireSet[]} */
  let sets;
  try {
    sets = /** @type {RequireSet[]} */ (
      await fetchJSON(`/admin/consumers/${encodeURIComponent(clientName)}/require-sets`)
    );
  } catch {
    card.remove();
    return;
  }
  if (sets.length === 0) {
    card.innerHTML =
      '<p class="text-xs text-slate-500">No versions of the require set yet: they come from trunk and tag requires of clients that send the hash of the <code>requires</code> section.</p>';
    return;
  }
  const columns = sets
    .map(
      (
        s,
      ) => `<div class="min-w-36 border border-slate-200 rounded-lg p-2" data-require-set="${String(s.number)}">
        <div class="text-xs font-semibold text-slate-700 mb-1">#${String(s.number)} <span class="font-normal text-slate-400">${escapeHtml(s.first_seen_at.slice(0, 10))}</span></div>
        <div class="flex flex-col gap-1">${s.pins
          .map(
            (p) =>
              `<span class="px-2 py-0.5 rounded-full text-[11px] font-mono bg-slate-50 text-slate-700 border border-slate-200">${escapeHtml(p.producer)} ${escapeHtml(p.version)}</span>`,
          )
          .join("")}</div>
      </div>`,
    )
    .join("");
  card.innerHTML = `<h3 class="font-semibold text-slate-800 mb-2">What each version requires</h3>
    <div class="flex gap-2 overflow-x-auto pb-1">${columns}</div>`;
}
