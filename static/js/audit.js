let timelineData = [];

async function loadTimeline() {
  showLoader();
  try {
    const from = document.getElementById("filter-from").value;
    const to = document.getElementById("filter-to").value;
    const type = document.getElementById("filter-type").value;
    const service = document.getElementById("filter-service").value.trim();
    const branch = document.getElementById("filter-branch").value.trim();

    let url = "/api/audit/timeline?limit=100";
    if (from) url += `&from_date=${from}`;
    if (to) url += `&to_date=${to}`;
    if (type) url += `&action_type=${type}`;
    if (service) url += `&service=${encodeURIComponent(service)}`;
    if (branch) url += `&branch=${encodeURIComponent(branch)}`;

    const res = await apiCall(url);
    if (!res.ok) {
      throw new Error("Failed to fetch timeline");
    }
    timelineData = await res.json();
    renderTimeline(timelineData);
  } catch (err) {
    console.error(err);
    document.getElementById("audit-timeline").innerHTML =
      `<div class="text-center py-20 text-red-500">Error loading timeline: ${escapeHtml(err.message)}</div>`;
  } finally {
    hideLoader();
  }
}

function renderTimeline(logs) {
  const container = document.getElementById("audit-timeline");
  if (logs.length === 0) {
    container.innerHTML =
      '<div class="text-center py-20 text-slate-400">No matching audit logs found.</div>';
    return;
  }

  container.innerHTML = logs
    .map((log) => {
      const date = new Date(log.timestamp);
      const shortDate = date.toLocaleString([], {
        month: "short",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });

      let actionColor = "bg-slate-100 text-slate-600";
      // Checked first: a rejection is a refusal, not a write, and must not be
      // coloured like one.
      if (log.action_type === "REJECT") actionColor = "bg-red-100 text-red-700";
      else if (log.action_type === "WRITE") actionColor = "bg-green-100 text-green-700";
      else if (log.action_type === "READ") actionColor = "bg-blue-100 text-blue-700";
      else if (log.action_type === "ADMIN") actionColor = "bg-amber-100 text-amber-700";
      else if (
        log.action === "PROVIDE_SPEC" ||
        log.action === "PROVIDE_ASYNCAPI" ||
        log.action === "PROVIDE_PROTO"
      )
        actionColor = "bg-green-100 text-green-700";
      else if (
        log.action === "REQUIRE_SPEC" ||
        log.action === "REQUIRE_ASYNCAPI" ||
        log.action === "REQUIRE_PROTO"
      )
        actionColor = "bg-blue-100 text-blue-700";

      return `
            <div class="timeline-item">
                <div class="bg-white rounded-lg border border-slate-200 shadow-sm overflow-hidden">
                    <div class="p-3 flex flex-wrap items-center justify-between gap-4">
                        <div class="flex items-center flex-wrap gap-x-4 gap-y-2 flex-1">
                            <span class="text-[11px] font-mono text-slate-400 min-w-[100px]">${shortDate}</span>
                            <span class="px-2 py-0.5 rounded text-[10px] font-bold uppercase tracking-wider ${actionColor}">${escapeHtml(log.action)}</span>
                            <span class="font-semibold text-sm text-slate-800">${escapeHtml(log.username)}</span>
                            <div class="flex items-center gap-2">
                                ${
                                  log.service
                                    ? `<span class="px-2 py-0.5 bg-indigo-50 text-indigo-700 rounded text-[10px] font-bold border border-indigo-100">${escapeHtml(log.service)}</span>`
                                    : ""
                                }
                                ${
                                  log.branch
                                    ? `<span class="px-2 py-0.5 bg-slate-50 text-slate-600 rounded text-[10px] font-mono border border-slate-100">${escapeHtml(log.branch)}</span>`
                                    : ""
                                }
                            </div>
                            <span class="text-sm text-slate-500 line-clamp-1 flex-1 min-w-[200px]" title="${escapeHtml(log.details)}">${escapeHtml(log.details)}</span>
                        </div>
                        
                        ${
                          log.diff
                            ? `
                            <button onclick="toggleDiff(${log.id})" class="text-indigo-600 hover:text-indigo-800 text-xs font-semibold flex items-center gap-1 whitespace-nowrap">
                                <svg id="diff-icon-${log.id}" class="w-3.5 h-3.5 transition-transform" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"></path>
                                </svg>
                                Changes
                            </button>
                        `
                            : ""
                        }
                    </div>
                    ${
                      log.diff
                        ? `
                        <div id="diff-container-${log.id}" class="hidden border-t border-slate-100 bg-slate-50">
                            <div id="diff-content-${log.id}" class="diff-view text-xs"></div>
                        </div>
                    `
                        : ""
                    }
                </div>
            </div>
        `;
    })
    .join("");
}

function toggleDiff(id) {
  const container = document.getElementById(`diff-container-${id}`);
  const icon = document.getElementById(`diff-icon-${id}`);
  const content = document.getElementById(`diff-content-${id}`);

  const isHidden = container.classList.contains("hidden");

  if (isHidden) {
    container.classList.remove("hidden");
    icon.style.transform = "rotate(180deg)";
    if (content.innerHTML === "") {
      renderDiffContent(id);
    }
  } else {
    container.classList.add("hidden");
    icon.style.transform = "rotate(0deg)";
  }
}

function renderDiffContent(id) {
  const log = timelineData.find((v) => v.id === id);
  if (!log || !log.diff) return;

  const diffContent = document.getElementById(`diff-content-${id}`);

  try {
    const diffHtml = Diff2Html.html(log.diff, {
      drawFileList: false,
      matching: "lines",
      outputFormat: "side-by-side",
      renderNothingWhenEmpty: false,
    });
    diffContent.innerHTML = diffHtml;
  } catch (err) {
    console.error("Diff rendering failed:", err);
    diffContent.innerHTML = '<div class="p-4 text-red-500 text-sm">Failed to render diff.</div>';
  }
}

function resetFilters() {
  document.getElementById("audit-filters").reset();
  loadTimeline();
}

// Attach filter form handler
function initAuditFilters() {
  const filterForm = document.getElementById("audit-filters");
  if (filterForm) {
    filterForm.addEventListener("submit", (e) => {
      e.preventDefault();
      loadTimeline();
    });
  }
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initAuditFilters);
} else {
  initAuditFilters();
}
