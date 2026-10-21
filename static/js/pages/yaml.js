// Extracted from static/yaml.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// The blame trail across a version line, oldest first. In endpoint mode an
// entry's yaml_content is one endpoint's snippet, and null marks versions
// that do not include the endpoint (absent — a deliberate omission). In
// full-spec mode it is the whole stored document, and null marks a version
// whose document could not be loaded.
let globalVersions = [];
let ticketTemplate = null;

// The origin of a version as one line of text, for a tooltip.
function originText(origin) {
  if (!origin) return "";
  const parts = [];
  if (origin.note) parts.push(origin.note);
  if (origin.change_url) parts.push(`change: ${origin.change_url}`);
  if (origin.build_url) parts.push(`build: ${origin.build_url}`);
  if ((origin.tickets || []).length) parts.push(`tickets: ${origin.tickets.join(", ")}`);
  return parts.join(" · ");
}
let activeVersion = null;
let isDiffMode = false;
let isBlameOn = false;

// Retrieve endpoint identity from query params
const params = new URLSearchParams(window.location.search);
const serviceName = params.get("service");
const apiType = (params.get("api_type") || "openapi").toLowerCase();
const endpointPath = params.get("path");
const endpointMethod = params.get("method");
let queryVersion = params.get("version") || null;
let queryCompareVersion = params.get("compare_version") || null;

// DOM bindings
const breadcrumbService = document.getElementById("breadcrumb-service");
const breadcrumbVersion = document.getElementById("breadcrumb-version");
const endpointMethodEl = document.getElementById("endpoint-method");
const endpointPathEl = document.getElementById("endpoint-path");

const versionsListContainer = document.getElementById("versions-list-container");
const diffFromSelect = document.getElementById("diff-from");
const diffToSelect = document.getElementById("diff-to");
const compareBtn = document.getElementById("compare-btn");

const viewYamlBtn = document.getElementById("view-yaml-btn");
const viewDiffBtn = document.getElementById("view-diff-btn");
const blameToggle = document.getElementById("blame-toggle");
const blameToggleContainer = document.getElementById("blame-toggle-container");
const copyBtn = document.getElementById("copy-btn");
const downloadBtn = document.getElementById("download-btn");
const viewerContainer = document.getElementById("viewer-container");

// Full-spec mode: no endpoint identity in the URL — the page shows the
// whole stored document of each version instead of one endpoint's snippet.
const isFullSpecMode = !endpointPath && !endpointMethod;

// Populate Breadcrumbs & Title
if (serviceName && (isFullSpecMode || (endpointPath && endpointMethod))) {
  breadcrumbService.textContent = serviceName;
  breadcrumbVersion.textContent = queryVersion ? `${apiType} ${queryVersion}` : apiType;
  document.getElementById("back-link").href =
    `/producers.html?service=${encodeURIComponent(serviceName)}`;
  if (isFullSpecMode) {
    document.title = "Sanshain - Full Spec Viewer";
    endpointMethodEl.textContent = apiType;
    endpointMethodEl.className =
      "px-2.5 py-0.5 rounded-sm text-sm font-semibold uppercase bg-purple-100 text-purple-700";
    endpointPathEl.textContent = "Full specification";
  } else {
    endpointMethodEl.textContent = endpointMethod;
    endpointPathEl.textContent = endpointPath;
    // Method badge colors based on method
    const colors = {
      GET: "bg-green-100 text-green-800",
      POST: "bg-blue-100 text-indigo-800",
      PUT: "bg-amber-100 text-amber-800",
      DELETE: "bg-red-100 text-red-800",
      PATCH: "bg-purple-100 text-purple-800",
    };
    const mClass = colors[endpointMethod.toUpperCase()] || "bg-slate-100 text-slate-800";
    endpointMethodEl.className = `px-2.5 py-0.5 rounded-sm text-sm font-semibold uppercase ${mClass}`;
  }
} else {
  alert("Missing endpoint identity parameters.");
  window.location.href = "/producers.html";
}

// Auth load
checkDiscoveryAuth(async () => {
  await renderBanner();
  await (isFullSpecMode ? loadFullSpecVersions() : loadEndpointVersions());
});

function attribution(v) {
  return v.provided_by || "anonymous";
}

async function loadEndpointVersions() {
  try {
    const res = await apiCall(
      `/admin/endpoint-versions?producername=${encodeURIComponent(serviceName)}&api_type=${encodeURIComponent(apiType)}&path=${encodeURIComponent(endpointPath)}&method=${encodeURIComponent(endpointMethod)}`,
    );
    // 404 = the Producer's line has no such version at all. Not a
    // transport failure — hand it to the single-version lookup, which
    // asks the server for the resolution state and renders the answer.
    const noHistory = res.status === 404;
    if (!noHistory && !res.ok) {
      throw new Error(`Fetch error: ${res.status}`);
    }
    const versions = noHistory ? [] : await res.json();
    if (!versions || versions.length === 0) {
      await loadSingleVersionFallback();
      return;
    }

    // Oldest first, semantically — the blame trail walks the line.
    globalVersions = [...versions].sort((a, b) => compareSemver(a.version, b.version));
    presentVersions();
  } catch (e) {
    console.error(e);
    hideLoader();
    viewerContainer.innerHTML = `<div class="text-red-400 font-semibold">Failed to load versions: ${escapeHtml(e.message)}</div>`;
  }
}

// Full-spec mode data: the line's version metadata plus each version's
// stored document, fetched in parallel. The endpoint viewer's rendering
// (sidebar, diff, blame) then works on whole documents instead of snippets.
async function loadFullSpecVersions() {
  try {
    const res = await apiCall(
      `/admin/producers/${encodeURIComponent(serviceName)}/versions?api_type=${encodeURIComponent(apiType)}`,
    );
    if (!res.ok) {
      throw new Error(`Fetch error: ${res.status}`);
    }
    const allLines = await res.json();
    const line = (allLines || []).filter(
      (v) => (v.api_type || "openapi").toLowerCase() === apiType,
    );
    if (line.length === 0) {
      hideLoader();
      viewerContainer.innerHTML = `<div class="text-slate-400 italic">This Producer has no stored ${escapeHtml(apiType)} versions.</div>`;
      return;
    }
    line.sort((a, b) => compareSemver(a.version, b.version));
    const contents = await Promise.all(
      line.map(async (v) => {
        const specRes = await apiCall(
          `/admin/producers/${encodeURIComponent(serviceName)}/full-spec?api_type=${encodeURIComponent(apiType)}&version=${encodeURIComponent(v.version)}`,
        );
        return specRes.ok ? await specRes.text() : null;
      }),
    );
    ticketTemplate = await loadTicketUrlTemplate();
    globalVersions = line.map((v, i) => ({
      version: v.version,
      stability: v.stability,
      provided_by: v.provided_by,
      updated_at: v.updated_at,
      origin: v.origin,
      yaml_content: contents[i],
      // A promotion keeps the content; only a new hash is a change.
      changed: i === 0 || v.content_hash !== line[i - 1].content_hash,
    }));
    presentVersions();
  } catch (e) {
    console.error(e);
    hideLoader();
    viewerContainer.innerHTML = `<div class="text-red-400 font-semibold">Failed to load versions: ${escapeHtml(e.message)}</div>`;
  }
}

// Shared tail of both loaders, once globalVersions is populated
// oldest-first: diff selectors, default selection, first render.
function presentVersions() {
  // Populate selectors for diffing (all entries; an absent version
  // diffs as an empty document, which shows the removal).
  const optionsHtml = globalVersions
    .map(
      (v) =>
        `<option value="${escapeHtml(v.version)}">${escapeHtml(v.version)} (${v.stability === "ga" ? "GA" : "snapshot"})</option>`,
    )
    .join("");
  diffFromSelect.innerHTML = optionsHtml;
  diffToSelect.innerHTML = optionsHtml;

  // Select default values: the queried version, else the newest
  // version that carries content, else the newest entry.
  const withContent = globalVersions.filter((v) => v.yaml_content != null);
  activeVersion =
    (queryVersion && globalVersions.find((v) => v.version === queryVersion)) ||
    withContent[withContent.length - 1] ||
    globalVersions[globalVersions.length - 1];

  if (queryCompareVersion) {
    diffFromSelect.value = queryCompareVersion;
    diffToSelect.value = activeVersion.version;
    isDiffMode = true;
  }

  renderVersionsSidebar();
  renderActiveView();
  hideLoader();
}

// When the blame trail yields nothing, ask for the pinned version directly:
// the server states how it resolved (served / absent / unknown), so the
// view renders that answer rather than inferring one from an empty body.
async function loadSingleVersionFallback() {
  const showMessage = (html) => {
    viewerContainer.innerHTML = `<div class="text-slate-400 italic">${html}</div>`;
    hideLoader();
  };
  if (!queryVersion) {
    showMessage("No version of this Producer includes this endpoint.");
    return;
  }
  try {
    const res = await apiCall(
      `/admin/endpoint-yaml?producername=${encodeURIComponent(serviceName)}&version=${encodeURIComponent(queryVersion)}&api_type=${encodeURIComponent(apiType)}&path=${encodeURIComponent(endpointPath)}&method=${encodeURIComponent(endpointMethod)}`,
    );
    if (!res.ok) {
      showMessage(`Could not load this endpoint (HTTP ${res.status}).`);
      return;
    }
    const view = await res.json();
    const version = escapeHtml(queryVersion);
    if (view.state === "absent") {
      showMessage(
        `Version ${version} exists but does not include this endpoint — a Require for it answers 410.`,
      );
      return;
    }
    if (view.state === "unknown" || !view.yaml) {
      showMessage(
        `The ${escapeHtml(apiType)} line of this Producer has no version ${version} in either stability — a Require pinned to it answers 404.`,
      );
      return;
    }
    globalVersions = [
      {
        version: view.version || queryVersion,
        stability: view.stability || "snapshot",
        provided_by: "",
        updated_at: "",
        yaml_content: view.yaml,
        changed: true,
      },
    ];
    activeVersion = globalVersions[0];
    diffFromSelect.innerHTML = "";
    diffToSelect.innerHTML = "";
    renderVersionsSidebar();
    renderActiveView();
    hideLoader();
  } catch (e) {
    console.error(e);
    hideLoader();
    viewerContainer.innerHTML = `<div class="text-red-400 font-semibold">Failed to load spec: ${escapeHtml(e.message)}</div>`;
  }
}

function renderVersionsSidebar() {
  const cards = [...globalVersions]
    .reverse()
    .map((v) => {
      const isActive = !isDiffMode && activeVersion && activeVersion.version === v.version;
      const isAbsent = v.yaml_content == null;
      const changedBadge = v.changed
        ? '<span class="text-[10px] bg-indigo-100 text-indigo-800 font-bold px-1.5 py-0.5 rounded-full uppercase">changed</span>'
        : "";
      const absentBadge = isAbsent
        ? `<span class="text-[10px] bg-slate-100 text-slate-500 font-bold px-1.5 py-0.5 rounded-full uppercase">${isFullSpecMode ? "unavailable" : "absent"}</span>`
        : "";
      return `
                <div data-version="${escapeHtml(v.version)}" class="version-card cursor-pointer border rounded-xl p-3.5 transition-all bg-white hover:border-indigo-300 ${isActive ? "border-indigo-600 bg-indigo-50/20 ring-2 ring-indigo-600/10" : "border-slate-200"} ${isAbsent ? "opacity-60" : ""}">
                    <div class="flex items-center justify-between mb-1.5 gap-1 flex-wrap">
                        <span class="text-sm font-bold text-slate-800 font-mono">${escapeHtml(v.version)}</span>
                        <span class="flex items-center gap-1">${stabilityBadge(v.stability)}${changedBadge}${absentBadge}</span>
                    </div>
                    <div class="text-xs text-slate-500 space-y-0.5">
                        <div class="flex items-center gap-1">
                            <svg class="w-3.5 h-3.5 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"/></svg>
                            <span>${escapeHtml(attribution(v))}</span>
                        </div>
                        <div class="flex items-center gap-1">
                            <svg class="w-3.5 h-3.5 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"/></svg>
                            <span>${escapeHtml((v.updated_at || "").slice(0, 16).replace("T", " "))}</span>
                        </div>
                    </div>
                </div>
            `;
    })
    .join("");
  versionsListContainer.innerHTML = cards;
  versionsListContainer.querySelectorAll(".version-card").forEach((card) => {
    card.addEventListener("click", () => selectVersion(card.dataset.version));
  });
}

function selectVersion(version) {
  isDiffMode = false;
  activeVersion = globalVersions.find((v) => v.version === version);
  updateUrlParams();
  renderVersionsSidebar();
  renderActiveView();
}

function renderActiveView() {
  updateToolbarState();

  if (isDiffMode) {
    renderDiffView();
  } else {
    renderYamlView();
  }
  breadcrumbVersion.textContent = activeVersion ? `${apiType} ${activeVersion.version}` : apiType;
}

function updateToolbarState() {
  if (isDiffMode) {
    viewYamlBtn.className =
      "px-3.5 py-1.5 text-xs font-semibold rounded-md transition-all text-slate-600 hover:text-slate-800";
    viewDiffBtn.className =
      "px-3.5 py-1.5 text-xs font-semibold rounded-md transition-all bg-white text-slate-800 shadow-xs";
    blameToggleContainer.classList.add("hidden");
  } else {
    viewYamlBtn.className =
      "px-3.5 py-1.5 text-xs font-semibold rounded-md transition-all bg-white text-slate-800 shadow-xs";
    viewDiffBtn.className =
      "px-3.5 py-1.5 text-xs font-semibold rounded-md transition-all text-slate-600 hover:text-slate-800";
    blameToggleContainer.classList.remove("hidden");
    blameToggle.checked = isBlameOn;
  }

  // Disable diff tab if only 1 version exists
  if (globalVersions.length < 2) {
    viewDiffBtn.disabled = true;
    viewDiffBtn.title = "Requires at least 2 versions to diff";
    viewDiffBtn.classList.add("opacity-50", "cursor-not-allowed");
  } else {
    viewDiffBtn.disabled = false;
    viewDiffBtn.title = "";
    viewDiffBtn.classList.remove("opacity-50", "cursor-not-allowed");
  }
}

function contentOf(v) {
  return v && v.yaml_content != null ? v.yaml_content : "";
}

function renderYamlView() {
  if (!activeVersion) return;
  if (activeVersion.yaml_content == null) {
    viewerContainer.innerHTML = isFullSpecMode
      ? `<div class="text-slate-400 italic">The stored document for version ${escapeHtml(activeVersion.version)} could not be loaded.</div>`
      : `<div class="text-slate-400 italic">Version ${escapeHtml(activeVersion.version)} does not include this endpoint — a Require for it answers 410.</div>`;
    return;
  }
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const lines = activeVersion.yaml_content.split("\n");

  if (isBlameOn) {
    renderBlameView(activeVersion);
  } else {
    let html = `<div class="flex items-start bg-slate-950 p-4 border border-slate-800 rounded-lg overflow-x-auto select-text">`;
    // Line numbers column
    html += `<div class="w-12 shrink-0 text-slate-500 text-right pr-3 border-r border-slate-800 select-none">`;
    for (let i = 1; i <= lines.length; i++) {
      html += `<div>${i}</div>`;
    }
    html += `</div>`;
    // Content column
    html += `<div class="pl-4 text-slate-200 flex-grow">`;
    for (const line of lines) {
      html += `<div>${esc(line)}</div>`;
    }
    html += `</div></div>`;
    viewerContainer.innerHTML = html;
  }
}

// Query parameters selecting what the server diffs: one endpoint's snippet
// in endpoint mode, the whole stored document in full-spec mode.
function diffTarget() {
  return isFullSpecMode ? {} : { path: endpointPath, method: endpointMethod };
}

// The patch for Copy and Download: the server's unified diff, whitespace
// included so it applies. Fetched whenever the diff view renders, so the Copy
// click can write it to the clipboard without awaiting anything first (a
// browser may refuse clipboard writes once the click's user activation is
// spent).
let diffPatch = { key: "", text: null, error: null };

function prefetchPatch(fromVersion, toVersion) {
  const state = { key: `${fromVersion}|${toVersion}`, text: null, error: null };
  diffPatch = state;
  fetchVersionDiff(serviceName, apiType, fromVersion, toVersion, diffTarget())
    .then((text) => {
      state.text = text;
    })
    .catch((e) => {
      state.error = e.message;
    });
}

// The prefetched patch for the selected pair, or null (with a toast saying
// why) when it is not available.
function readyPatch() {
  const key = `${diffFromSelect.value}|${diffToSelect.value}`;
  if (diffPatch.key !== key || (diffPatch.text == null && !diffPatch.error)) {
    showToast("The diff is still loading — try again in a moment.");
    return null;
  }
  if (diffPatch.error) {
    showToast(`Could not load the diff: ${diffPatch.error}`);
    return null;
  }
  return diffPatch.text;
}

function renderDiffView() {
  const fromVersion = diffFromSelect.value;
  const toVersion = diffToSelect.value;
  if (!fromVersion || !toVersion) return;

  prefetchPatch(fromVersion, toVersion);
  const originOf = (version) => {
    const v = globalVersions.find((x) => x.version === version);
    const html = v ? originHtml(v.origin, ticketTemplate, true) : "";
    return html
      ? `<div class="mt-1 text-[11px]" data-diff-origin="${escapeHtml(version)}"><span class="text-slate-400">${escapeHtml(version)}:</span>${html}</div>`
      : "";
  };
  mountDiffView(viewerContainer, {
    caption: `Comparing <span class="text-indigo-400 font-bold">${escapeHtml(fromVersion)}</span> &rarr; <span class="text-indigo-400 font-bold">${escapeHtml(toVersion)}</span>${originOf(fromVersion)}${originOf(toVersion)}`,
    emptyHtml: `No differences between ${escapeHtml(fromVersion)} and ${escapeHtml(toVersion)}.`,
    // Endpoint snippets are re-serialized when a spec is split, so they never
    // differ in whitespace alone: the switch would have no effect there.
    ignoreWhitespaceSwitch: isFullSpecMode,
    load: (ignoreWhitespace) =>
      fetchVersionDiff(serviceName, apiType, fromVersion, toVersion, {
        ...diffTarget(),
        format: "json",
        context: "all",
        ignore_whitespace: String(ignoreWhitespace),
      }),
  });
}

viewYamlBtn.onclick = () => {
  isDiffMode = false;
  updateUrlParams();
  renderActiveView();
};

viewDiffBtn.onclick = () => {
  if (globalVersions.length < 2) return;
  isDiffMode = true;
  // set compare versions to standard values if they aren't configured
  if (diffFromSelect.value === diffToSelect.value) {
    diffFromSelect.value = globalVersions[0].version;
    diffToSelect.value = globalVersions[globalVersions.length - 1].version;
  }
  updateUrlParams();
  renderActiveView();
};

blameToggle.onchange = (e) => {
  isBlameOn = e.target.checked;
  renderActiveView();
};

compareBtn.onclick = () => {
  isDiffMode = true;
  updateUrlParams();
  renderActiveView();
};

// Copy Content to Clipboard
// Copy Content to Clipboard. Synchronous up to the clipboard write; when the
// Clipboard API is missing or refuses, fall back to selecting the text in a
// temporary textarea and copying that.
copyBtn.onclick = () => {
  let textToCopy = "";
  if (isDiffMode) {
    const patch = readyPatch();
    if (patch == null) return;
    if (!patch) {
      showToast("No differences to copy.");
      return;
    }
    textToCopy = patch;
  } else if (activeVersion) {
    textToCopy = contentOf(activeVersion);
  }
  if (textToCopy) copyText(textToCopy);
};

function copyText(text) {
  const fallback = () => {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    document.body.appendChild(textarea);
    textarea.select();
    const copied = document.execCommand("copy");
    document.body.removeChild(textarea);
    showToast(copied ? "Copied to clipboard!" : "Copy failed — select the text manually.");
  };
  if (!navigator.clipboard || !navigator.clipboard.writeText) {
    fallback();
    return;
  }
  navigator.clipboard.writeText(text).then(() => showToast("Copied to clipboard!"), fallback);
}

// Download / Export as file
downloadBtn.onclick = () => {
  // Endpoint snippets are always YAML; a full proto document is not.
  const fileExt = isFullSpecMode && apiType === "proto" ? "proto" : "yaml";
  let contentStr = "";
  let filename = `${serviceName}.${fileExt}`;

  if (isDiffMode) {
    const patch = readyPatch();
    if (patch == null) return;
    contentStr = patch;
    filename = `${serviceName}_${diffFromSelect.value}_to_${diffToSelect.value}.patch`;
  } else if (activeVersion) {
    contentStr = contentOf(activeVersion);
    filename = `${serviceName}_${activeVersion.version}.${fileExt}`;
  }

  if (contentStr) {
    const blob = new Blob([contentStr], { type: isDiffMode ? "text/plain" : "application/x-yaml" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }
};

// Blame view: the server walks the line with the same diff as the diff views
// and answers blocks of consecutive lines last changed in the same version.
// Each block shows user, version and date once, on its first line; a thin
// gutter bar spans the block and neighbouring blocks alternate background.
async function renderBlameView(version) {
  const q = new URLSearchParams({ api_type: apiType, version: version.version, ...diffTarget() });
  let blocks;
  try {
    const res = await apiCall(`/admin/producers/${encodeURIComponent(serviceName)}/blame?${q}`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    blocks = await res.json();
  } catch (e) {
    if (activeVersion === version && isBlameOn && !isDiffMode) {
      viewerContainer.innerHTML = `<div class="text-red-400 font-semibold">Failed to load blame: ${escapeHtml(e.message)}</div>`;
    }
    return;
  }
  // A newer selection or mode replaced this view while the request ran.
  if (activeVersion !== version || !isBlameOn || isDiffMode) return;

  const html = blocks
    .map((b, bi) => {
      const user = attribution(b);
      const shortUser = user.length > 12 ? user.slice(0, 10) + ".." : user;
      const shade = bi % 2 === 0 ? "bg-slate-950" : "bg-slate-900";
      const why = originText(b.origin);
      const meta = `
          <span class="font-bold text-indigo-400 shrink-0 mr-1" ${why ? `title="${escapeHtml(why)}" data-origin-title` : ""}>${escapeHtml(b.version)}</span>
          <span class="truncate max-w-[70px]" title="${escapeHtml(user)}">${escapeHtml(shortUser)}</span>
          <span class="opacity-70 text-[10px] font-light shrink-0 ml-1">${escapeHtml((b.updated_at || "").slice(0, 10))}</span>`;
      const lines = b.lines
        .map(
          (text, li) => `
          <div class="blame-line flex items-stretch">
            <div class="blame-meta w-48 shrink-0 text-slate-400 px-3 py-0.5 border-r border-slate-800 text-[11px] flex items-center justify-between select-none">${li === 0 ? meta : ""}</div>
            <div class="w-12 shrink-0 text-slate-500 text-right pr-2.5 py-0.5 border-r border-slate-800 select-none">${b.start_line + li}</div>
            <div class="grow pl-3 py-0.5 text-slate-200 whitespace-pre">${escapeHtml(text)}</div>
          </div>`,
        )
        .join("");
      return `<div class="blame-block grid grid-cols-1 border-l-2 border-indigo-500/70 ${shade}" data-version="${escapeHtml(b.version)}">${lines}</div>`;
    })
    .join("");
  viewerContainer.innerHTML = blocks.length
    ? `<div class="border border-slate-800 rounded-lg overflow-x-auto bg-slate-950 font-mono text-xs select-text">${html}</div>`
    : `<div class="text-slate-400 italic">Nothing to blame: this version's text is empty.</div>`;
}

function updateUrlParams() {
  const urlParams = new URLSearchParams();
  urlParams.set("service", serviceName);
  urlParams.set("api_type", apiType);
  if (!isFullSpecMode) {
    urlParams.set("path", endpointPath);
    urlParams.set("method", endpointMethod);
  }

  if (isDiffMode) {
    urlParams.set("compare_version", diffFromSelect.value);
    urlParams.set("version", diffToSelect.value);
  } else if (activeVersion) {
    urlParams.set("version", activeVersion.version);
  }

  history.pushState({}, "", `${window.location.pathname}?${urlParams.toString()}`);
}

function showToast(message) {
  const toast = document.createElement("div");
  toast.className =
    "fixed bottom-5 right-5 bg-slate-800 text-white px-4 py-2 rounded-lg shadow-lg z-50 text-sm font-medium transition-opacity";
  toast.textContent = message;
  document.body.appendChild(toast);
  setTimeout(() => {
    toast.style.opacity = "0";
    setTimeout(() => toast.remove(), 300);
  }, 2000);
}

window.addEventListener("sanshain-update", () => {
  console.log("Live update received, refreshing versions...");
  if (isFullSpecMode) {
    loadFullSpecVersions();
  } else {
    loadEndpointVersions();
  }
});
