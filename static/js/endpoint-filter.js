// The search box and chips over an endpoint list (producers.html version
// view, consumers.html dependency view). The filter lives in the URL
// (`q`, `show`), so Back from an endpoint returns to the filtered list and a
// link can be shared. Every word of `q` must match the path, the method or
// the summary; `show` holds at most one of used/unused plus deprecated.

const ENDPOINT_FILTER_USE = ["used", "unused"];

function endpointFilterFromUrl() {
  const params = new URLSearchParams(window.location.search);
  const show = new Set((params.get("show") || "").split(",").filter(Boolean));
  return { q: params.get("q") || "", show };
}

function endpointFilterToUrl(filter) {
  const params = new URLSearchParams(window.location.search);
  const q = filter.q.trim();
  if (q) params.set("q", q);
  else params.delete("q");
  const show = [...ENDPOINT_FILTER_USE, "deprecated"].filter((s) => filter.show.has(s));
  if (show.length) params.set("show", show.join(","));
  else params.delete("show");
  const search = params.toString();
  // Replace, not push: typing must not fill the history with one entry per key.
  history.replaceState(history.state, "", search ? `?${search}` : window.location.pathname);
}

function endpointFilterActive(filter) {
  return filter.q.trim() !== "" || filter.show.size > 0;
}

// `ep` carries method, path, summary, deprecated and, on the producer view,
// `used`.
function endpointFilterMatches(ep, filter) {
  if (filter.show.has("deprecated") && !ep.deprecated) return false;
  if (filter.show.has("used") && !ep.used) return false;
  if (filter.show.has("unused") && ep.used) return false;
  const haystack = `${ep.method} ${ep.path} ${ep.summary || ""}`.toLowerCase();
  return filter.q
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => haystack.includes(word));
}

// Draw the search box and `chips` (a subset of "used", "unused",
// "deprecated"; "All" comes with used/unused) into `container`. `onChange`
// runs after every change, once the URL holds the new filter.
function renderEndpointFilter(container, filter, chips, onChange) {
  const chipCls = (on) =>
    `px-2.5 py-1 rounded-full text-xs font-semibold border ${on ? "bg-indigo-600 text-white border-indigo-600" : "bg-white text-slate-600 border-slate-300 hover:bg-slate-50"}`;
  const labels = { all: "All", used: "Used", unused: "Unused", deprecated: "Deprecated" };
  const names = chips.includes("used") ? ["all", ...chips] : chips;
  container.className = "flex flex-wrap items-center gap-2";
  container.innerHTML = `
      <input type="search" data-endpoint-filter-q placeholder="Filter by path, method or summary…"
        class="flex-1 min-w-48 px-3 py-1.5 rounded-lg border border-slate-300 text-sm focus:outline-hidden focus:ring-2 focus:ring-indigo-500" />
      ${names.map((name) => `<button type="button" data-endpoint-filter-chip="${name}">${labels[name]}</button>`).join("")}`;
  const input = container.querySelector("[data-endpoint-filter-q]");
  input.value = filter.q;

  const paintChips = () => {
    container.querySelectorAll("[data-endpoint-filter-chip]").forEach((button) => {
      const name = button.dataset.endpointFilterChip;
      const on =
        name === "all"
          ? !ENDPOINT_FILTER_USE.some((s) => filter.show.has(s))
          : filter.show.has(name);
      button.className = chipCls(on);
      button.setAttribute("aria-pressed", String(on));
    });
  };
  const changed = () => {
    endpointFilterToUrl(filter);
    paintChips();
    onChange();
  };

  input.addEventListener("input", () => {
    filter.q = input.value;
    changed();
  });
  container.addEventListener("click", (e) => {
    const button = e.target.closest("[data-endpoint-filter-chip]");
    if (!button) return;
    const name = button.dataset.endpointFilterChip;
    if (name === "deprecated") {
      if (filter.show.has(name)) filter.show.delete(name);
      else filter.show.add(name);
    } else {
      const wasOn = filter.show.has(name);
      ENDPOINT_FILTER_USE.forEach((s) => filter.show.delete(s));
      if (name !== "all" && !wasOn) filter.show.add(name);
    }
    changed();
  });
  paintChips();

  return {
    clear() {
      filter.q = "";
      filter.show.clear();
      input.value = "";
      changed();
    },
  };
}

// The "nothing matches" line with its Clear filter link.
function endpointFilterEmptyHtml() {
  return `<div class="text-center py-10 text-slate-500 italic" data-endpoint-filter-empty>No endpoints match this filter. <button type="button" class="not-italic font-semibold text-indigo-600 hover:text-indigo-800" data-endpoint-filter-clear>Clear filter</button></div>`;
}
