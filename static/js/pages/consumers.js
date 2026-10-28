// Extracted from static/consumers.html: inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

let allClients = [];

async function showClients() {
  showLoader();
  const list = document.getElementById("clients-list");
  list.innerHTML = '<div class="text-center py-10 text-slate-500">Loading consumers...</div>';

  try {
    await loadUserFavorites();
    const clients = await fetchJSON("/admin/consumers");
    allClients = clients || [];
    document.getElementById("client-search").value = "";
    renderClientList(allClients);
    hideLoader();
  } catch (err) {
    hideLoader();
    list.innerHTML = `<div class="text-red-500 text-center py-10">Error: ${escapeHtml(err.message)}</div>`;
  }
}

function goBackToClients() {
  history.pushState({}, "", window.location.pathname);
  showClients();
}

function renderClientList(clients) {
  const list = document.getElementById("clients-list");
  if (!clients || clients.length === 0) {
    list.innerHTML = `
          <div class="bg-white p-10 rounded-2xl border-2 border-dashed border-slate-200 text-center">
              <div class="text-slate-500 mb-4">
                  <svg class="w-16 h-16 mx-auto" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z"></path>
                  </svg>
              </div>
              <h3 class="text-lg font-medium text-slate-900">No consumers found</h3>
              <p class="text-slate-500 mt-1">Consumers will appear here once they request endpoints via GET /require.</p>
          </div>`;
    return;
  }
  list.innerHTML = "";
  for (const name of clients) {
    const isFav = !!(userFavorites.clients && userFavorites.clients.includes(name));
    const card = document.createElement("div");
    card.className =
      "bg-white p-5 rounded-xl border border-slate-200 shadow-xs endpoint-card cursor-pointer";
    card.onclick = () => {
      history.pushState({ name }, "", `?name=${encodeURIComponent(name)}`);
      showClientDependencies(name);
    };
    // Consumer names are client-supplied and unvalidated: set as text,
    // bind handlers as listeners, never splice into markup.
    card.innerHTML = `
          <div class="flex items-center justify-between">
              <div class="flex items-center">
                  <span class="bg-teal-100 text-teal-700 p-2 rounded-lg mr-3">
                      <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"></path>
                      </svg>
                  </span>
                  <div class="flex items-center gap-2">
                      <h3 class="font-bold text-lg text-slate-800" data-client-name></h3>
                      <button type="button" data-toggle-favorite class="text-amber-400 hover:text-amber-500 p-1 focus:outline-hidden rounded-sm transition-colors" title="${isFav ? "Remove from favorites" : "Mark as favorite"}">
                          ${
                            isFav
                              ? `
                              <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 20 20"><path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z"/></svg>
                          `
                              : `
                              <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z"/></svg>
                          `
                          }
                      </button>
                  </div>
              </div>
              <svg class="w-5 h-5 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path>
              </svg>
          </div>`;
    card.querySelector("[data-client-name]").textContent = name;
    card
      .querySelector("[data-toggle-favorite]")
      .addEventListener("click", (event) => toggleFavorite(event, "client", name, isFav));
    list.appendChild(card);
  }
}

// A Consumer's dependencies, grouped by the Producer they pin. Each group
// header names the Pin (version + stability); each row is one endpoint.
async function showClientDependencies(clientName) {
  showLoader();
  const list = document.getElementById("clients-list");
  list.innerHTML = '<div class="text-center py-10 text-slate-500">Loading dependencies...</div>';

  try {
    const endpoints = await fetchJSON(
      `/admin/consumers/${encodeURIComponent(clientName)}/endpoints`,
    );
    list.innerHTML = "";

    const breadcrumb = document.createElement("div");
    breadcrumb.className = "flex items-center text-sm text-slate-500 mb-4";
    breadcrumb.innerHTML = `
          <button data-click="goBackToClients" class="hover:text-teal-600 font-medium">Consumers</button>
          <svg class="w-4 h-4 mx-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path></svg>
          <span class="text-slate-800 font-semibold" data-crumb-client></span>`;
    breadcrumb.querySelector("[data-crumb-client]").textContent = clientName;
    list.appendChild(breadcrumb);

    // Owner, description, repository and links from the Consumer's
    // catalog-info (#48), under the title. Optional: the page reads without it.
    try {
      const { context } = await fetchJSON(`/admin/consumers/${encodeURIComponent(clientName)}`);
      const block = systemContextHtml(context, false);
      if (block) {
        const card = document.createElement("div");
        card.className = "bg-white p-4 rounded-xl border border-slate-200 shadow-xs";
        card.innerHTML = block;
        list.appendChild(card);
      }
    } catch (_) {
      /* the context is optional */
    }

    // The Consumer's webhooks (#47): the events of everything it requires,
    // managed by a Maintainer of the Producer of the same name or an admin.
    const webhooks = document.createElement("div");
    webhooks.className = "bg-white p-4 rounded-xl border border-slate-200 shadow-xs hidden";
    webhooks.innerHTML =
      '<h3 class="font-semibold text-slate-800 mb-1">Webhooks</h3><p class="text-xs text-slate-500 mb-3">Signed POSTs for the events of every Producer this Consumer currently requires — the hook follows the require graph.</p>';
    const webhooksPanel = document.createElement("div");
    webhooks.appendChild(webhooksPanel);
    list.appendChild(webhooks);
    renderWebhookPanel(
      webhooksPanel,
      `/admin/consumers/${encodeURIComponent(clientName)}/webhooks`,
    ).then((visible) => {
      if (visible) webhooks.classList.remove("hidden");
    });

    if (!endpoints || endpoints.length === 0) {
      const empty = document.createElement("div");
      empty.className = "text-center py-10 text-slate-500 italic";
      empty.textContent = "No dependencies recorded for this Consumer.";
      list.appendChild(empty);
      hideLoader();
      return;
    }

    const filterCard = document.createElement("div");
    filterCard.className = "bg-white p-4 rounded-xl border border-slate-200 shadow-xs";
    list.appendChild(filterCard);
    const filterRows = [];
    const filterSections = [];

    // Group by (producer, api_type, pinned version) — one Pin per group.
    const groups = new Map();
    for (const ep of endpoints) {
      const key = `${ep.service} ${ep.api_type} ${ep.version}`;
      if (!groups.has(key))
        groups.set(key, {
          service: ep.service,
          api_type: ep.api_type,
          version: ep.version,
          stability: ep.stability,
          endpoints: [],
        });
      groups.get(key).endpoints.push(ep);
    }
    const sortedGroups = [...groups.values()].sort(
      (a, b) =>
        a.service.localeCompare(b.service) ||
        String(a.api_type).localeCompare(String(b.api_type)) ||
        compareSemver(a.version, b.version),
    );

    for (const group of sortedGroups) {
      const section = document.createElement("div");
      section.className = "bg-white p-4 rounded-xl border border-slate-200 shadow-xs";

      const header = document.createElement("div");
      header.className = "flex items-center justify-between gap-2 flex-wrap mb-3";
      header.innerHTML = `
              <div class="flex items-center gap-2 flex-wrap">
                  <span class="bg-indigo-100 text-indigo-700 p-1.5 rounded-lg">
                      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"></path>
                      </svg>
                  </span>
                  <h3 class="font-semibold text-slate-800" data-group-service></h3>
                  <span class="px-1.5 py-0.5 rounded-md text-[9px] font-bold uppercase tracking-wider bg-slate-100 text-slate-500 border border-slate-200">${escapeHtml(group.api_type || "REST")}</span>
                  <span class="text-xs text-slate-500">pinned to</span>
                  <span class="font-mono text-sm text-slate-700">${escapeHtml(group.version)}</span>
                  ${stabilityBadge(group.stability)}
              </div>
              <button type="button" data-view-producer class="text-xs font-semibold text-indigo-600 hover:text-indigo-800">View version →</button>`;
      header.querySelector("[data-group-service]").textContent = group.service;
      header.querySelector("[data-view-producer]").addEventListener("click", () => {
        window.location.href = `/producers.html?service=${encodeURIComponent(group.service)}&api_type=${encodeURIComponent(group.api_type)}&version=${encodeURIComponent(group.version)}`;
      });
      section.appendChild(header);

      const rows = document.createElement("div");
      rows.className = "space-y-2";
      for (const ep of group.endpoints) {
        const row = document.createElement("div");
        const resolved = !!ep.yaml_content;
        row.className =
          "bg-slate-50 p-3 rounded-lg border border-slate-100 endpoint-card" +
          (resolved ? " cursor-pointer" : "");
        if (resolved) {
          row.onclick = () => {
            window.location.href = `/yaml.html?service=${encodeURIComponent(ep.service)}&api_type=${encodeURIComponent(ep.api_type || "openapi")}&version=${encodeURIComponent(ep.version)}&path=${encodeURIComponent(ep.path)}&method=${encodeURIComponent(ep.method)}`;
          };
        }
        row.innerHTML = `
                  <div class="flex items-center justify-between">
                      <div class="flex items-center">
                          <span class="px-2 py-1 rounded-sm text-[10px] font-bold uppercase mr-3 ${getMethodColor(ep.method)}">
                              ${escapeHtml(ep.method)}
                          </span>
                          <span class="font-mono text-sm text-slate-700">${escapeHtml(ep.path)}</span>
                          ${ep.deprecated ? '<span class="text-[10px] text-amber-600 bg-amber-100 px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter ml-2">deprecated</span>' : ""}
                          ${ep.external ? '<span class="text-[10px] text-emerald-600 bg-emerald-100 px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter ml-2">external</span>' : ""}
                      </div>
                      ${ep.summary ? `<div class="endpoint-summary text-xs text-slate-500 mt-1.5 ml-1">${escapeHtml(ep.summary)}</div>` : ""}
                      <div class="flex items-center">
                          ${
                            resolved
                              ? '<span class="text-xs text-green-600 bg-green-50 px-2 py-1 rounded-full mr-2">served</span><svg class="w-5 h-5 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path></svg>'
                              : '<span class="text-xs text-red-600 bg-red-50 px-2 py-1 rounded-full mr-2" title="The pinned version no longer serves this endpoint — the Consumer\'s next require will fail.">not served</span>'
                          }
                      </div>
                  </div>`;
        rows.appendChild(row);
        filterRows.push({ ep, row, section });
      }
      section.appendChild(rows);
      list.appendChild(section);
      filterSections.push(section);
    }

    // A group with no matching endpoint hides with its rows.
    const empty = document.createElement("div");
    empty.innerHTML = endpointFilterEmptyHtml();
    list.appendChild(empty);
    const filter = endpointFilterFromUrl();
    const apply = () => {
      const visible = new Set();
      for (const { ep, row, section } of filterRows) {
        const match = endpointFilterMatches(ep, filter);
        row.classList.toggle("hidden", !match);
        if (match) visible.add(section);
      }
      for (const section of filterSections)
        section.classList.toggle("hidden", !visible.has(section));
      empty.classList.toggle("hidden", visible.size > 0);
    };
    const control = renderEndpointFilter(filterCard, filter, ["deprecated"], apply);
    empty
      .querySelector("[data-endpoint-filter-clear]")
      .addEventListener("click", () => control.clear());
    apply();
    hideLoader();
  } catch (err) {
    hideLoader();
    list.innerHTML = `<div class="text-red-500 text-center py-10">Error: ${escapeHtml(err.message)}</div>`;
  }
}

document.getElementById("client-search").addEventListener("input", function () {
  const q = this.value.toLowerCase();
  if (!q) {
    renderClientList(allClients);
    return;
  }
  renderClientList(allClients.filter((name) => name.toLowerCase().includes(q)));
});

async function handleDeepLink() {
  const params = new URLSearchParams(window.location.search);
  const name = params.get("name");

  if (name) {
    await showClientDependencies(name);
  } else {
    await showClients();
  }
}

// Initial load
checkDiscoveryAuth(async () => {
  await handleDeepLink();
});

// Handle back/forward navigation
window.addEventListener("popstate", () => {
  handleDeepLink();
});

window.addEventListener("sanshain-update", () => {
  console.log("Live update received, refreshing consumers...");
  handleDeepLink();
});
