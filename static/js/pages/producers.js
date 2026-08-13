// Extracted from static/producers.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// The signed-in caller, for gating affordances (the Promote button).
// Hiding is a usability affordance; the server's GA gate is the boundary.
let currentUser = null;
// version-key -> sanshain-branch names referencing it (reverse lookup).
let branchMembershipMap = new Map();

async function loadServices() {
  showLoader();
  const list = document.getElementById("services-list");
  const breadcrumb = document.getElementById("service-breadcrumb");
  breadcrumb.classList.add("hidden");
  breadcrumb.innerHTML = "";
  document.getElementById("service-search").value = "";
  list.innerHTML = '<div class="text-center py-10">Loading services...</div>';

  try {
    await loadUserFavorites();
    await loadAllProducers();
    renderServiceList(allServices);
    hideLoader();
  } catch (err) {
    hideLoader();
    list.innerHTML = `<div class="text-red-500 text-center py-10">Error: ${escapeHtml(err.message)}</div>`;
  }
}

function goBackToServices() {
  history.pushState({}, "", window.location.pathname);
  loadServices();
}

function goBackToVersions(serviceName) {
  history.pushState({ service: serviceName }, "", `?service=${encodeURIComponent(serviceName)}`);
  showProducerVersions(serviceName);
}

function renderServiceList(allServicesIn) {
  const list = document.getElementById("services-list");
  const breadcrumb = document.getElementById("service-breadcrumb");
  breadcrumb.classList.add("hidden");
  breadcrumb.innerHTML = "";

  // Only show Producers that serve something: at least one entry of their
  // version lines must have at least one endpoint. A Producer whose only
  // versions are empty (e.g. an OpenAPI spec provided with no paths)
  // provides no benefit to list here.
  const services = (allServicesIn || []).filter((svc) => producerHasAnyEndpoints(svc.name));

  if (!services || services.length === 0) {
    list.innerHTML = `
                <div class="bg-white p-10 rounded-2xl border-2 border-dashed border-slate-200 text-center">
                    <div class="text-slate-500 mb-4">
                        <svg class="w-16 h-16 mx-auto" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"></path>
                        </svg>
                    </div>
                    <h3 class="text-lg font-medium text-slate-900">No services found</h3>
                    <p class="text-slate-500 mt-1">Provide an OpenAPI spec via POST /provide to see it here.</p>
                </div>`;
    return;
  }

  list.innerHTML = "";
  for (const svc of services) {
    const name = svc.name;
    const versions = (svc.versions || []).filter((v) => (v.endpoint_count || 0) > 0);
    const isFav = !!svc.is_favorite;
    const icon = svc.icon || null;
    const domain = svc.domain || null;
    const card = document.createElement("div");
    card.className =
      "bg-white p-5 rounded-xl border border-slate-200 shadow-sm endpoint-card cursor-pointer";
    card.onclick = () => {
      history.pushState({ service: name }, "", `?service=${encodeURIComponent(name)}`);
      showProducerVersions(name);
    };
    card.innerHTML = `
                <div class="flex items-center justify-between">
                    <div class="flex items-center">
                        <span class="bg-indigo-100 text-indigo-700 p-2 rounded-lg mr-3 flex items-center justify-center w-10 h-10">
                            ${
                              icon
                                ? `<span class="text-xl">${escapeHtml(icon)}</span>`
                                : `
                            <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"></path>
                            </svg>`
                            }
                        </span>
                        <div>
                            <div class="flex items-center gap-2">
                                <h3 class="font-bold text-lg text-slate-800">${escapeHtml(name)}</h3>
                                ${domain ? `<span class="text-[10px] bg-blue-50 text-blue-600 px-2 py-0.5 rounded-full border border-blue-100 font-bold uppercase">${escapeHtml(domain)}</span>` : ""}
                                <button type="button" data-toggle-favorite class="text-amber-400 hover:text-amber-500 p-1 focus:outline-none rounded transition-colors" title="${isFav ? "Remove from favorites" : "Mark as favorite"}">
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
                            <div class="text-sm text-slate-500">
                                <span class="w-2 h-2 inline-block rounded-full bg-green-500 mr-1"></span>
                                ${versions.length} version${versions.length !== 1 ? "s" : ""}
                            </div>
                        </div>
                    </div>
                    <svg class="w-5 h-5 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path>
                    </svg>
                </div>`;
    // Bound as a listener rather than an `onclick` attribute so the
    // service name (client-supplied, unvalidated) is never spliced into
    // markup.
    card
      .querySelector("[data-toggle-favorite]")
      .addEventListener("click", (event) => toggleFavorite(event, "service", name, isFav));
    list.appendChild(card);
  }
}

document.getElementById("service-search").addEventListener("input", function () {
  const q = this.value.toLowerCase();
  if (!q) {
    renderServiceList(allServices);
    return;
  }
  renderServiceList(allServices.filter((svc) => svc.name.toLowerCase().includes(q)));
});

const API_TYPE_ORDER = ["openapi", "asyncapi", "proto"];

function formatTimestamp(iso) {
  return iso ? iso.slice(0, 16).replace("T", " ") : "";
}

// The version timeline: one list per API type, newest first.
async function showProducerVersions(serviceName) {
  hideLoader();
  const list = document.getElementById("services-list");
  const breadcrumb = document.getElementById("service-breadcrumb");
  breadcrumb.classList.remove("hidden");
  breadcrumb.innerHTML = `
            <button data-click="goBackToServices" class="hover:text-indigo-600 font-medium">Producers</button>
            <svg class="w-4 h-4 mx-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path></svg>
            <span class="text-slate-800 font-semibold">${escapeHtml(serviceName)}</span>`;

  list.innerHTML = '<div class="text-center py-10 text-slate-500">Loading versions...</div>';

  let versions;
  try {
    versions = await fetchJSON(`/admin/producers/${encodeURIComponent(serviceName)}/versions`);
  } catch (err) {
    list.innerHTML = `<div class="text-red-500 text-center py-10">Error: ${escapeHtml(err.message)}</div>`;
    return;
  }

  // Reverse lookup (ADR-0005): which sanshain-branches reference each
  // version. Optional decoration — the listing renders without it.
  branchMembershipMap = new Map();
  try {
    const memberships = await fetchJSON(
      `/admin/producers/${encodeURIComponent(serviceName)}/branch-memberships`,
    );
    for (const m of memberships) {
      const key = `${(m.api_type || "openapi").toLowerCase()}|${m.version}`;
      if (!branchMembershipMap.has(key)) branchMembershipMap.set(key, []);
      branchMembershipMap.get(key).push(m.branch);
    }
  } catch (_) {
    /* chips are optional */
  }
  list.innerHTML = "";

  if (!versions || versions.length === 0) {
    list.innerHTML =
      '<div class="text-center py-10 text-slate-500 italic">No versions found for this Producer.</div>';
    return;
  }

  // Group into version lines by API type.
  const lines = new Map();
  for (const v of versions) {
    const key = (v.api_type || "openapi").toLowerCase();
    if (!lines.has(key)) lines.set(key, []);
    lines.get(key).push(v);
  }
  const orderedTypes = [...lines.keys()].sort((a, b) => {
    const ia = API_TYPE_ORDER.indexOf(a),
      ib = API_TYPE_ORDER.indexOf(b);
    return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib);
  });

  for (const apiType of orderedTypes) {
    const line = lines.get(apiType);
    line.sort((a, b) => compareSemver(b.version, a.version)); // newest first
    list.appendChild(renderVersionLine(serviceName, apiType, line));
  }
}

function renderVersionLine(serviceName, apiType, line) {
  const section = document.createElement("div");
  section.className = "bg-white p-4 rounded-xl border border-slate-200 shadow-sm";

  const header = document.createElement("div");
  header.className = "flex flex-wrap items-center justify-between gap-3 mb-3";
  header.innerHTML = `
            <div class="flex items-center gap-2">
                <span class="px-2 py-0.5 rounded-md text-[10px] font-bold uppercase tracking-wider bg-slate-100 text-slate-600 border border-slate-200">${escapeHtml(apiType)}</span>
                <span class="text-sm text-slate-500">${line.length} version${line.length !== 1 ? "s" : ""}</span>
            </div>`;

  // Diff control: any two versions of the line, defaulting to the two
  // newest (adjacent) entries.
  if (line.length >= 2) {
    const diffCtl = document.createElement("div");
    diffCtl.className = "flex items-center gap-2 text-sm";
    const mkSelect = () => {
      const sel = document.createElement("select");
      sel.className = "text-sm border border-slate-300 rounded-lg px-2 py-1";
      for (const v of line) {
        const opt = document.createElement("option");
        opt.value = v.version;
        opt.textContent = `${v.version} (${v.stability === "ga" ? "GA" : "snapshot"})`;
        sel.appendChild(opt);
      }
      return sel;
    };
    const fromSel = mkSelect();
    const toSel = mkSelect();
    fromSel.value = line[1].version; // second-newest
    toSel.value = line[0].version; // newest
    const arrow = document.createElement("span");
    arrow.className = "text-slate-500";
    arrow.textContent = "→";
    const diffBtn = document.createElement("button");
    diffBtn.type = "button";
    diffBtn.className =
      "px-3 py-1 text-sm font-medium bg-indigo-600 text-white rounded-lg hover:bg-indigo-700";
    diffBtn.textContent = "Diff";
    diffBtn.addEventListener("click", () =>
      showLineDiff(serviceName, apiType, fromSel.value, toSel.value, diffOutput),
    );
    diffCtl.append(fromSel, arrow, toSel, diffBtn);
    header.appendChild(diffCtl);
  }
  section.appendChild(header);

  const diffOutput = document.createElement("div");
  diffOutput.className = "mb-3 empty:mb-0";
  section.appendChild(diffOutput);

  const entries = document.createElement("div");
  entries.className = "space-y-2";
  for (const v of line) {
    entries.appendChild(renderVersionEntry(serviceName, apiType, v));
  }
  section.appendChild(entries);
  return section;
}

function renderVersionEntry(serviceName, apiType, v) {
  const isSnapshot = v.stability !== "ga";
  let expiryBadge = "";
  if (isSnapshot && v.expires_at) {
    const daysLeft = Math.ceil((new Date(v.expires_at).getTime() - Date.now()) / 86400000);
    const label = daysLeft <= 0 ? "Expiring" : `Expires in ${daysLeft}d`;
    const urgent = daysLeft <= 3;
    expiryBadge = `<span class="text-[10px] ${urgent ? "text-red-700 bg-red-100" : "text-amber-700 bg-amber-100"} px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter" title="This snapshot expires once neither provided nor required for the configured window (${escapeHtml(v.expires_at)}).">${label}</span>`;
  }
  const provider = v.provided_by;

  const card = document.createElement("div");
  card.className =
    "bg-white p-4 rounded-xl border border-slate-200 shadow-sm endpoint-card cursor-pointer";
  card.onclick = () => {
    history.pushState(
      { service: serviceName, api_type: apiType, version: v.version },
      "",
      `?service=${encodeURIComponent(serviceName)}&api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(v.version)}`,
    );
    showVersionEndpoints(serviceName, apiType, v.version);
  };
  card.innerHTML = `
            <div class="flex items-center justify-between gap-3 flex-wrap">
                <div class="flex items-center gap-3 min-w-0">
                    <span class="bg-purple-100 text-purple-700 p-2 rounded-lg">
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 7h.01M7 3h5a2 2 0 011.414.586l7 7a2 2 0 010 2.828l-5 5a2 2 0 01-2.828 0l-7-7A2 2 0 015 10V5a2 2 0 012-2z"></path>
                        </svg>
                    </span>
                    <div class="min-w-0">
                        <div class="flex items-center gap-2 flex-wrap">
                            <h3 class="font-semibold text-slate-800 font-mono">${escapeHtml(v.version)}</h3>
                            ${stabilityBadge(v.stability)}
                            ${v.trunk_provided_at ? `<span class="text-[10px] bg-emerald-100 text-emerald-700 px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter" title="Trunk CI last provided this version ${escapeHtml(v.trunk_provided_at)}">trunk</span>` : ""}
                            ${(branchMembershipMap.get(`${apiType}|${v.version}`) || []).map((b) => `<span class="text-[10px] bg-sky-100 text-sky-700 px-2 py-0.5 rounded-full font-bold tracking-tighter" title="Referenced by sanshain-branch ${escapeHtml(b)}">${escapeHtml(b)}</span>`).join("")}
                            ${expiryBadge}
                        </div>
                        <p class="text-xs text-slate-400 mt-0.5">
                            ${v.endpoint_count} endpoint${v.endpoint_count !== 1 ? "s" : ""}
                            &middot; <span data-provider></span>
                            &middot; updated ${escapeHtml(formatTimestamp(v.updated_at))}
                        </p>
                    </div>
                </div>
                <div class="flex items-center gap-2">
                    <button type="button" data-view class="px-2.5 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-indigo-600">View spec</button>
                    <button type="button" data-download class="px-2.5 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-indigo-600">Download</button>
                    ${
                      isSnapshot && hasPermission(currentUser, "release_ga")
                        ? '<button type="button" data-promote class="px-2.5 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-green-50 text-green-700">Promote to GA</button>'
                        : ""
                    }
                    <button type="button" data-delete class="px-2.5 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-red-50 text-red-600">Delete</button>
                    <svg class="w-5 h-5 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path>
                    </svg>
                </div>
            </div>`;
  // Author is Consumer-supplied, unverified attribution — set as text,
  // never spliced into markup.
  card.querySelector("[data-provider]").textContent = `by ${provider}`;
  card.querySelector("[data-view]").addEventListener("click", (e) => {
    e.stopPropagation();
    // No path/method params — yaml.html opens in full-spec mode.
    window.location.href = `/yaml.html?service=${encodeURIComponent(serviceName)}&api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(v.version)}`;
  });
  card.querySelector("[data-download]").addEventListener("click", (e) => {
    e.stopPropagation();
    downloadFullApi(serviceName, apiType, v.version);
  });
  card.querySelector("[data-delete]").addEventListener("click", (e) => {
    e.stopPropagation();
    deleteVersion(serviceName, apiType, v.version);
  });
  const promoteBtn = card.querySelector("[data-promote]");
  if (promoteBtn) {
    promoteBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      // Shared flow from common.js; the callback is this page's refresh.
      promoteVersion(serviceName, apiType, v.version, async () => {
        await loadAllProducers();
        showProducerVersions(serviceName);
      });
    });
  }
  return card;
}

async function showLineDiff(serviceName, apiType, from, to, output) {
  if (from === to) {
    output.innerHTML = '<div class="text-sm text-slate-500 italic">Same version selected.</div>';
    return;
  }
  output.innerHTML = '<div class="text-sm text-slate-500 italic">Loading diff...</div>';
  try {
    const res = await apiCall(
      `/admin/producers/${encodeURIComponent(serviceName)}/diff?api_type=${encodeURIComponent(apiType)}&from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
    );
    if (!res.ok) {
      output.innerHTML = `<div class="text-sm text-red-500">Failed to load diff (HTTP ${res.status}).</div>`;
      return;
    }
    const diffText = await res.text();
    if (!diffText.trim() || diffText.trim().split("\n").length <= 1) {
      output.innerHTML = `<div class="text-sm text-slate-500 italic">No differences between ${escapeHtml(from)} and ${escapeHtml(to)}.</div>`;
      return;
    }
    output.innerHTML = `
                <div class="text-xs text-slate-500 mb-1">${escapeHtml(from)} → ${escapeHtml(to)}</div>
                <pre class="diff-pre bg-slate-800 p-4 rounded-xl text-sm overflow-x-auto whitespace-pre-wrap leading-relaxed">${renderUnifiedDiffHtml(diffText)}</pre>`;
  } catch (e) {
    output.innerHTML = `<div class="text-sm text-red-500">Failed to load diff: ${escapeHtml(e.message)}</div>`;
  }
}

// Delete-version is the sole escape hatch from GA immutability. Always
// offered; the server refuses callers without the permission (403). The
// dependents are fetched FIRST so the confirmation names who breaks.
async function deleteVersion(serviceName, apiType, version) {
  let dependents = [];
  try {
    dependents = await fetchJSON(
      `/admin/producers/${encodeURIComponent(serviceName)}/versions/${encodeURIComponent(apiType)}/${encodeURIComponent(version)}/dependents`,
    );
  } catch (e) {
    alert("Could not check dependents: " + e.message);
    return;
  }
  const base = `Delete ${escapeHtml(apiType)} version <strong>${escapeHtml(version)}</strong> of <strong>${escapeHtml(serviceName)}</strong>?`;
  const warning =
    dependents.length > 0
      ? `<br><br><span class="text-red-600 font-semibold">Pinned Consumers (their requires fail with 404) and referencing sanshain-branches (left dangling):</span><br>${dependents.map((d) => escapeHtml(d)).join(", ")}`
      : "<br><br>No Consumers are pinned to this version.";
  confirmDelete(base + warning, async () => {
    try {
      const res = await apiCall(
        `/admin/producers/${encodeURIComponent(serviceName)}/versions/${encodeURIComponent(apiType)}/${encodeURIComponent(version)}`,
        { method: "DELETE" },
      );
      if (!res.ok) {
        alert("Failed to delete: " + (await errorMessage(res)));
        return;
      }
      await loadAllProducers();
      showProducerVersions(serviceName);
    } catch (e) {
      alert("Failed to delete: " + e.message);
    }
  });
}

async function downloadFullApi(service, apiType, version) {
  try {
    const res = await apiCall(
      `/admin/producers/${encodeURIComponent(service)}/full-spec?api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(version)}`,
    );
    if (!res.ok) {
      alert("Failed to load the full API spec for this version.");
      return;
    }
    const text = await res.text();
    const blob = new Blob([text], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${service}-${version}.${apiType === "proto" ? "proto" : "yaml"}`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  } catch (e) {
    alert("Failed to download the full API: " + e.message);
  }
}

async function showVersionEndpoints(serviceName, apiType, version) {
  showLoader();
  const list = document.getElementById("services-list");
  const breadcrumb = document.getElementById("service-breadcrumb");
  breadcrumb.classList.remove("hidden");
  // Service names come straight from client `/provide` payloads and are
  // not validated, so they must never be interpolated raw into markup —
  // escape text, and wire behaviour through data-action attributes rather
  // than splicing names into inline handler attributes.
  breadcrumb.innerHTML = `
            <button data-click="goBackToServices" class="hover:text-indigo-600 font-medium">Producers</button>
            <svg class="w-4 h-4 mx-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path></svg>
            <button type="button" data-back-to-versions class="hover:text-indigo-600 font-medium">${escapeHtml(serviceName)}</button>
            <svg class="w-4 h-4 mx-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path></svg>
            <span class="text-slate-800 font-semibold font-mono">${escapeHtml(apiType)} ${escapeHtml(version)}</span>`;
  breadcrumb
    .querySelector("[data-back-to-versions]")
    .addEventListener("click", () => goBackToVersions(serviceName));

  list.innerHTML = '<div class="text-center py-10 text-slate-500">Loading endpoints...</div>';

  try {
    const providedEndpoints = await fetchJSON(
      `/admin/producers/${encodeURIComponent(serviceName)}/endpoints?api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(version)}`,
    );
    if (providedEndpoints && providedEndpoints.length > 0) {
      const dlBtn = document.createElement("button");
      dlBtn.type = "button";
      dlBtn.className =
        "ml-4 px-2.5 py-1 text-xs font-semibold bg-indigo-600 text-white rounded-lg hover:bg-indigo-700";
      dlBtn.textContent = "Download full API";
      dlBtn.addEventListener("click", () => downloadFullApi(serviceName, apiType, version));
      breadcrumb.appendChild(dlBtn);
    }
    let report = { dependency_graph: [], unused_endpoints: [] };
    try {
      report = await fetchJSON("/report");
    } catch (_) {}

    const uniqueEndpoints = (providedEndpoints || []).map((e) => ({
      api_type: e.api_type,
      path: e.path,
      method: e.method,
      service: serviceName,
      deprecated: e.deprecated,
    }));

    // Dependencies recorded against exactly this Pin of this line.
    const versionDeps = report.dependency_graph.filter(
      (e) =>
        e.service === serviceName &&
        String(e.version) === String(version) &&
        (e.api_type || "openapi").toLowerCase() === apiType.toLowerCase(),
    );

    const clientsMap = {};
    versionDeps.forEach((e) => {
      const key = `${e.method}:${e.path}`;
      if (!clientsMap[key]) clientsMap[key] = new Set();
      clientsMap[key].add(e.client);
    });

    const usedKeys = new Set(versionDeps.map((e) => `${e.method}:${e.path}`));
    const unusedSet = new Set(
      uniqueEndpoints
        .filter((e) => !usedKeys.has(`${e.method}:${e.path}`))
        .map((e) => `${e.method}:${e.path}`),
    );

    list.innerHTML = "";
    hideLoader();

    if (uniqueEndpoints.length === 0) {
      list.innerHTML =
        '<div class="text-center py-10 text-slate-500 italic">This version includes no endpoints.</div>';
      return;
    }

    const summary = document.createElement("div");
    summary.className =
      "bg-white p-4 rounded-xl border border-slate-200 shadow-sm mb-2 flex items-center justify-between";
    const usedCount = uniqueEndpoints.filter((e) => !unusedSet.has(`${e.method}:${e.path}`)).length;
    const unusedCount = uniqueEndpoints.length - usedCount;
    summary.innerHTML = `
                <div class="text-sm text-slate-600">
                    <span class="font-semibold text-slate-800">${uniqueEndpoints.length}</span> endpoints total &middot;
                    <span class="text-green-600 font-medium">${usedCount} used</span> &middot;
                    <span class="text-amber-600 font-medium">${unusedCount} unused</span>
                    <span class="text-slate-400">(at this Pin)</span>
                </div>`;
    list.appendChild(summary);

    for (const ep of uniqueEndpoints) {
      const key = `${ep.method}:${ep.path}`;
      const isUnused = unusedSet.has(key);
      const clients = clientsMap[key] ? [...clientsMap[key]] : [];
      const card = document.createElement("div");
      card.className =
        "bg-white p-4 rounded-xl border border-slate-200 shadow-sm endpoint-card cursor-pointer";
      // Delegated (not a direct card.onclick) so the client badge nested inside
      // can be its own data-click: the dispatcher fires only the nearest
      // data-click ancestor, so a badge click opens its popover without also
      // opening the endpoint — no stopPropagation needed.
      card.dataset.click = "openEndpointYaml";
      setActionArgs(card, "click", [serviceName, apiType, version, ep.path, ep.method]);
      card.innerHTML = `
                    <div class="flex items-center justify-between">
                        <div class="flex items-center">
                            <span class="px-1.5 py-0.5 rounded-md text-[9px] font-bold uppercase tracking-wider mr-2 bg-slate-100 text-slate-500 border border-slate-200">
                                ${escapeHtml(ep.api_type || "REST")}
                            </span>
                            <span class="px-2 py-1 rounded text-[10px] font-bold uppercase mr-3 ${getMethodColor(ep.method)}">
                                ${escapeHtml(ep.method)}
                            </span>
                            <span class="font-mono text-sm text-slate-700">${escapeHtml(ep.path)}</span>
                            ${ep.deprecated ? '<span class="text-[10px] text-amber-600 bg-amber-100 px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter ml-2">deprecated</span>' : ""}
                        </div>
                        <div class="flex items-center">
                            ${
                              isUnused
                                ? '<span class="text-xs text-amber-600 bg-amber-50 px-2 py-1 rounded-full mr-2">unused</span>'
                                : `<span class="text-xs text-green-600 bg-green-50 px-2 py-1 rounded-full mr-2 relative client-badge-wrapper" data-clients="${escapeHtml(clients.join(","))}" data-click="toggleClientPopover" data-click-args='["$this"]'>${clients.length} client${clients.length !== 1 ? "s" : ""} ▾</span>`
                            }
                            <svg class="w-5 h-5 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"></path>
                            </svg>
                        </div>
                    </div>`;
      list.appendChild(card);
    }
  } catch (err) {
    hideLoader();
    list.innerHTML = `<div class="text-red-500 text-center py-10">Error: ${escapeHtml(err.message)}</div>`;
  }
}

// Opens the endpoint's stored YAML — the endpoint card's delegated action
// (was card.onclick). A named global so the card's data-click resolves.
function openEndpointYaml(serviceName, apiType, version, path, method) {
  window.location.href = `/yaml.html?service=${encodeURIComponent(serviceName)}&api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(version)}&path=${encodeURIComponent(path)}&method=${encodeURIComponent(method)}`;
}

function toggleClientPopover(badge) {
  document.querySelectorAll(".client-popover").forEach((p) => p.remove());
  const clients = badge.dataset.clients.split(",").filter(Boolean);
  if (clients.length === 0) return;
  const popover = document.createElement("div");
  popover.className =
    "client-popover absolute right-0 top-full mt-1 bg-white border border-slate-200 rounded-lg shadow-lg z-50 py-1 min-w-[180px]";
  // Client names are client-supplied and unvalidated: build each entry as
  // a node with textContent and a bound listener, never as interpolated
  // markup.
  for (const c of clients) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className =
      "block w-full text-left px-3 py-1.5 text-sm text-slate-700 hover:bg-indigo-50 hover:text-indigo-700";
    btn.textContent = c;
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      navigateToClient(c);
    });
    popover.appendChild(btn);
  }
  badge.style.position = "relative";
  badge.appendChild(popover);
  setTimeout(() => {
    const handler = (e) => {
      if (!badge.contains(e.target)) {
        popover.remove();
        document.removeEventListener("click", handler);
      }
    };
    document.addEventListener("click", handler);
  }, 0);
}

function navigateToClient(clientName) {
  document.querySelectorAll(".client-popover").forEach((p) => p.remove());
  window.location.href = `/consumers.html?name=${encodeURIComponent(clientName)}`;
}

async function handleDeepLink() {
  const params = new URLSearchParams(window.location.search);
  const service = params.get("service");
  const apiType = params.get("api_type") || "openapi";
  const version = params.get("version");
  const path = params.get("path");
  const method = params.get("method");

  if (service && version && path && method) {
    window.location.href = `/yaml.html?service=${encodeURIComponent(service)}&api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(version)}&path=${encodeURIComponent(path)}&method=${encodeURIComponent(method)}`;
  } else if (service && version) {
    await showVersionEndpoints(service, apiType, version);
  } else if (service) {
    await showProducerVersions(service);
  } else {
    await loadServices();
  }
}

// Initial load
checkDiscoveryAuth(async (user) => {
  currentUser = user;
  await loadAllProducers();
  await handleDeepLink();
});

// Handle back/forward navigation
window.addEventListener("popstate", () => {
  handleDeepLink();
});

window.addEventListener("sanshain-update", () => {
  console.log("Live update received, refreshing services...");
  handleDeepLink();
});
