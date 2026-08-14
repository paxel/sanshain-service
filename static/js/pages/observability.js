// Extracted from static/observability.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

let isAdmin = false;

function onSessionExpired() {
  window.location.href = "/account.html";
}

async function checkSession() {
  if (!sanshainToken) {
    window.location.href = "/account.html";
    return;
  }
  try {
    const res = await apiCall("/auth/me");
    if (res.ok) {
      const data = await res.json();
      // Reading logs and stats needs only `view_observability`;
      // changing the debug configuration is a settings change.
      isAdmin = hasPermission(data, "manage_settings");
      await renderBanner(data);
      applyAdminState();
      hideLoader();
      loadObservability();
      loadBranchCount();
    } else {
      window.location.href = "/account.html";
    }
  } catch (_) {
    window.location.href = "/account.html";
  }
}

function applyAdminState() {
  const hint = document.getElementById("debug-admin-hint");
  const logicBtn = document.getElementById("toggle-debug-logic");
  const adminBtn = document.getElementById("toggle-debug-admin");
  const logicCard = document.getElementById("debug-logic-card");
  const adminCard = document.getElementById("debug-admin-card");

  if (!isAdmin) {
    hint.classList.remove("hidden");
    logicBtn.disabled = true;
    adminBtn.disabled = true;
    logicBtn.classList.add("opacity-40", "cursor-not-allowed");
    adminBtn.classList.add("opacity-40", "cursor-not-allowed");
    logicCard.classList.add("opacity-60");
    adminCard.classList.add("opacity-60");
  }
}

// --- Observability ---
async function loadObservability() {
  loadStats();
  loadDebugConfig();
  loadLogs();
  loadAuditLogs();
}

async function loadBranchCount() {
  // ADR-0005: the branch count gauge, human-readable next to the raw
  // metrics link (the per-branch update counter lives in /metrics).
  try {
    const res = await apiCall("/admin/branches");
    if (res.ok) {
      const branches = await res.json();
      document.getElementById("branch-count-stat").textContent =
        `${branches.length} sanshain-branch${branches.length === 1 ? "" : "es"}`;
    }
  } catch (_) {
    /* decorative */
  }
}

async function loadStats() {
  try {
    const res = await apiCall("/admin/observability/stats");
    if (res.ok) {
      const data = await res.json();
      document.getElementById("stat-cpu").textContent = `${(data.cpu_usage || 0).toFixed(1)}%`;
      document.getElementById("stat-mem").textContent =
        `${((data.memory_used || 0) / 1024 / 1024).toFixed(0)} / ${((data.memory_total || 0) / 1024 / 1024).toFixed(0)} MB`;
      document.getElementById("stat-requests").textContent = data.requests_total || 0;
      document.getElementById("stat-failures").textContent = data.failures_total || 0;
      document.getElementById("stat-uptime").textContent = formatUptime(data.process_uptime || 0);
    }
  } catch (_) {}
}

function formatUptime(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

let debugConfig = { business_logic_debug: false, admin_user_debug: false };

async function loadDebugConfig() {
  try {
    const res = await apiCall("/admin/observability/debug-config");
    if (res.ok) {
      debugConfig = await res.json();
      updateDebugUI();
    }
  } catch (_) {}
}

function updateDebugUI() {
  const logicToggle = document.getElementById("toggle-debug-logic");
  const logicDot = document.getElementById("dot-debug-logic");
  const adminToggle = document.getElementById("toggle-debug-admin");
  const adminDot = document.getElementById("dot-debug-admin");

  if (debugConfig.business_logic_debug) {
    logicToggle.classList.remove("bg-slate-200");
    logicToggle.classList.add("bg-indigo-600");
    logicDot.classList.remove("translate-x-1");
    logicDot.classList.add("translate-x-6");
  } else {
    logicToggle.classList.remove("bg-indigo-600");
    logicToggle.classList.add("bg-slate-200");
    logicDot.classList.remove("translate-x-6");
    logicDot.classList.add("translate-x-1");
  }

  if (debugConfig.admin_user_debug) {
    adminToggle.classList.remove("bg-slate-200");
    adminToggle.classList.add("bg-indigo-600");
    adminDot.classList.remove("translate-x-1");
    adminDot.classList.add("translate-x-6");
  } else {
    adminToggle.classList.remove("bg-indigo-600");
    adminToggle.classList.add("bg-slate-200");
    adminDot.classList.remove("translate-x-6");
    adminDot.classList.add("translate-x-1");
  }
}

async function toggleDebug(type) {
  if (!isAdmin) return;
  if (type === "logic") {
    debugConfig.business_logic_debug = !debugConfig.business_logic_debug;
  } else {
    debugConfig.admin_user_debug = !debugConfig.admin_user_debug;
  }
  try {
    await apiCall("/admin/observability/debug-config-update", {
      method: "POST",
      body: debugConfig,
    });
    updateDebugUI();
  } catch (_) {}
}

let currentLogs = { errors: [], warnings: [], infos: [], debugs: [] };

async function loadLogs() {
  try {
    const res = await apiCall("/admin/observability/logs");
    if (res.ok) {
      const data = await res.json();
      currentLogs.errors = data.errors || [];
      currentLogs.warnings = data.warnings || [];
      currentLogs.infos = data.infos || [];
      currentLogs.debugs = data.debugs || [];
      renderLogs();
    }
  } catch (_) {}
}

function renderLogs() {
  const container = document.getElementById("log-container");
  const filter = document.getElementById("log-filter").value;

  let errors = [...currentLogs.errors];
  let warnings = [...currentLogs.warnings];
  let infos = [...currentLogs.infos];
  let debugs = [...currentLogs.debugs];

  if (filter === "INFO") {
    debugs = [];
  } else if (filter === "WARN") {
    debugs = [];
    infos = [];
  } else if (filter === "ERROR") {
    debugs = [];
    infos = [];
    warnings = [];
  }

  const allLogs = [...errors, ...warnings, ...infos, ...debugs].sort((a, b) =>
    a.timestamp.localeCompare(b.timestamp),
  );

  if (allLogs.length === 0) {
    container.innerHTML = '<div class="text-slate-500 italic">No logs captured yet.</div>';
    return;
  }

  container.innerHTML = allLogs
    .map((l) => {
      const color =
        {
          ERROR: "text-red-400",
          WARN: "text-amber-400",
          INFO: "text-indigo-300",
          DEBUG: "text-slate-500",
          TRACE: "text-slate-600",
        }[l.level] || "text-slate-300";

      // Some events (provide/require) carry service/version context; show it as
      // a compact chip when present. Most lines (auth, startup, migrations)
      // have none, so the chip is omitted rather than padded with a placeholder.
      const context = l.service
        ? `<span class="text-teal-400">[${escapeHtml(l.service)}${l.version ? "@" + escapeHtml(l.version) : ""}]</span> `
        : "";
      return `<div class="mb-1"><span class="text-slate-500">[${l.timestamp.substring(11, 19)}]</span> <span class="${color} font-bold">${l.level.padEnd(5)}</span> <span class="text-indigo-400">${escapeHtml(l.target)}</span>: ${context}${escapeHtml(l.message)}</div>`;
    })
    .join("");

  const autoScroll = document.getElementById("log-autoscroll");
  if (autoScroll && autoScroll.checked) {
    container.scrollTop = container.scrollHeight;
  }
}

function getLogsAsText() {
  const filter = document.getElementById("log-filter").value;
  let errors = [...currentLogs.errors];
  let warnings = [...currentLogs.warnings];
  let infos = [...currentLogs.infos];
  let debugs = [...currentLogs.debugs];
  if (filter === "INFO") {
    debugs = [];
  } else if (filter === "WARN") {
    debugs = [];
    infos = [];
  } else if (filter === "ERROR") {
    debugs = [];
    infos = [];
    warnings = [];
  }
  const allLogs = [...errors, ...warnings, ...infos, ...debugs].sort((a, b) =>
    a.timestamp.localeCompare(b.timestamp),
  );
  // Same [service@version] context as the on-screen view: without it,
  // exported provide/require lines name no producer at all.
  return allLogs
    .map((l) => {
      const context = l.service ? `[${l.service}${l.version ? "@" + l.version : ""}] ` : "";
      return `[${l.timestamp.substring(11, 19)}] ${l.level.padEnd(5)} ${l.target}: ${context}${l.message}`;
    })
    .join("\n");
}

function copyLogs() {
  const text = getLogsAsText();
  if (!text) {
    return;
  }
  const btn = event.target.closest("button");
  const orig = btn.textContent;
  function onSuccess() {
    btn.textContent = "✓ Copied";
    setTimeout(() => {
      btn.textContent = orig;
    }, 1500);
  }
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard
      .writeText(text)
      .then(onSuccess)
      .catch(() => {
        fallbackCopy(text, onSuccess);
      });
  } else {
    fallbackCopy(text, onSuccess);
  }
}

function fallbackCopy(text, onSuccess) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.left = "-9999px";
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
    onSuccess();
  } catch (_) {
    alert("Failed to copy logs to clipboard.");
  } finally {
    ta.remove();
  }
}

function downloadLogs() {
  const text = getLogsAsText();
  if (!text) {
    return;
  }
  const blob = new Blob([text], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `sanshain-logs-${new Date().toISOString().slice(0, 19).replace(/:/g, "-")}.txt`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// Auto-refresh every 5s
setInterval(() => {
  loadStats();
  loadLogs();
  loadAuditLogs();
}, 5000);

// Fetch well past what is shown: rejections share this table, so a CI job
// retrying a breaking change used to push every real change out of a fixed
// 30-row window. Hiding them client-side then still leaves changes to see.
const AUDIT_PANEL_FETCH = 100;
const AUDIT_PANEL_SHOW = 30;

async function loadAuditLogs() {
  try {
    const res = await apiCall(`/admin/observability/audit-logs?limit=${AUDIT_PANEL_FETCH}`);
    if (res.ok) {
      const logs = await res.json();
      renderAuditLogs(logs);
    } else if (res.status === 403) {
      const body = document.getElementById("audit-log-body");
      body.innerHTML =
        '<tr><td colspan="5" class="p-4 text-center text-slate-400 italic">Admin access required to view the audit log.</td></tr>';
    }
  } catch (_) {}
}

function renderAuditLogs(logs) {
  const body = document.getElementById("audit-log-body");
  const hideRejections = document.getElementById("audit-hide-rejections")?.checked;
  const all = logs || [];
  const rejections = all.filter((l) => l.action_type === "REJECT").length;
  const shown = (hideRejections ? all.filter((l) => l.action_type !== "REJECT") : all).slice(
    0,
    AUDIT_PANEL_SHOW,
  );
  const note = document.getElementById("audit-rejection-note");
  if (note) {
    note.textContent = rejections
      ? `${rejections} rejection${rejections === 1 ? "" : "s"} in the last ${AUDIT_PANEL_FETCH} entries`
      : "";
  }
  if (shown.length === 0) {
    body.innerHTML =
      '<tr><td colspan="5" class="p-4 text-center text-slate-400 italic">No audit logs found.</td></tr>';
    return;
  }

  body.innerHTML = shown
    .map((log) => {
      let actionBadgeColor = "bg-slate-100 text-slate-700";
      if (log.action === "NUKE_DATABASE") {
        actionBadgeColor = "bg-red-100 text-red-700";
      } else if (log.action === "DELETE_VERSION") {
        // The audited escape hatch from GA immutability — destructive
        // for any Consumer pinned to the deleted version.
        actionBadgeColor = "bg-orange-100 text-orange-700";
      } else if (log.action === "VERSION_REJECTED") {
        actionBadgeColor = "bg-rose-100 text-rose-700";
      } else if (log.action === "PROVIDE_SPEC" || log.action === "VERSION_PROMOTED") {
        actionBadgeColor = "bg-green-100 text-green-700";
      } else if (
        log.action === "UPDATE_SETTINGS" ||
        log.action === "SET_DEV_MODE" ||
        log.action === "SET_LOCAL_USERS" ||
        log.action === "SET_AUTO_APPROVE"
      ) {
        actionBadgeColor = "bg-amber-100 text-amber-700";
      } else if (
        log.action === "APPROVE_USER" ||
        log.action === "REGISTER_USER" ||
        log.action === "DELETE_USER"
      ) {
        actionBadgeColor = "bg-indigo-100 text-indigo-700";
      }

      return `
                <tr class="hover:bg-slate-50 transition-colors">
                    <td class="p-3 text-slate-400 font-mono">${log.id}</td>
                    <td class="p-3 text-slate-500 font-mono whitespace-nowrap">${log.timestamp.substring(0, 19).replace("T", " ")}</td>
                    <td class="p-3 font-semibold text-slate-700">${escapeHtml(log.username)}</td>
                    <td class="p-3"><span class="px-2 py-0.5 rounded-full text-[10px] font-bold uppercase tracking-wider ${actionBadgeColor}">${escapeHtml(log.action)}</span></td>
                    <td class="p-3 text-slate-600">${escapeHtml(log.details)}</td>
                </tr>
            `;
    })
    .join("");
}

async function exportAuditLogs() {
  try {
    const res = await apiCall("/admin/observability/audit-logs/export");
    if (res.ok) {
      const blob = await res.blob();
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "audit_logs.csv";
      document.body.appendChild(a);
      a.click();
      a.remove();
      window.URL.revokeObjectURL(url);
    } else {
      alert("Failed to export audit logs. Unauthorized or server error.");
    }
  } catch (e) {
    alert("Error exporting audit logs: " + e);
  }
}

checkSession();
