// Extracted from static/observability.html: inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// What common.js calls when the session ended, and what the controls of the page name in their
// data-click attributes.
/* exported onSessionExpired, toggleDebug, copyLogs, downloadLogs, exportAuditLogs */

/**
 * `SystemStats` in `src/domain/models.rs`: `GET /admin/observability/stats`.
 * @typedef {object} ObservabilityStats
 * @property {number} [cpu_usage]
 * @property {number} [memory_used]
 * @property {number} [memory_total]
 * @property {number} [process_uptime]
 * @property {number} [requests_total]
 * @property {number} [failures_total]
 */

/**
 * `DebugConfig` in `src/domain/models.rs`.
 * @typedef {object} ObservabilityDebugConfig
 * @property {boolean} business_logic_debug
 * @property {boolean} admin_user_debug
 */

/**
 * `LogEntry` in `src/domain/models.rs`.
 * @typedef {object} ObservabilityLogEntry
 * @property {string} timestamp
 * @property {string} level
 * @property {string} target
 * @property {string} message
 * @property {string | null} [service]
 * @property {string | null} [version]
 */

/** @typedef {"errors" | "warnings" | "infos" | "debugs"} ObservabilityLogLevel */

/**
 * `AuditLogEntry` in `src/domain/models.rs`, as far as the panel reads it.
 * @typedef {object} ObservabilityAuditEntry
 * @property {number} id
 * @property {string} timestamp
 * @property {string} username
 * @property {string} action
 * @property {string} details
 * @property {string | null} [action_type]
 */

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
      const data = /** @type {SanshainUser} */ (await res.json());
      // Reading logs and stats needs only `view_observability`;
      // changing the debug configuration is a settings change.
      isAdmin = hasPermission(data, "manage_settings");
      canAudit = (data.is_root ?? false) || hasPermission(data, "view_audit");
      await renderBanner(data);
      applyAdminState();
      hideLoader();
      loadObservability();
      void loadBranchCount();
    } else {
      window.location.href = "/account.html";
    }
  } catch {
    window.location.href = "/account.html";
  }
}

function applyAdminState() {
  const hint = /** @type {HTMLElement} */ (document.getElementById("debug-admin-hint"));
  const logicBtn = /** @type {HTMLButtonElement} */ (document.getElementById("toggle-debug-logic"));
  const adminBtn = /** @type {HTMLButtonElement} */ (document.getElementById("toggle-debug-admin"));
  const logicCard = /** @type {HTMLElement} */ (document.getElementById("debug-logic-card"));
  const adminCard = /** @type {HTMLElement} */ (document.getElementById("debug-admin-card"));

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
function loadObservability() {
  void loadStats();
  void loadDebugConfig();
  void loadLogs();
  void loadAuditLogs();
}

// The audit log needs `view_audit`; without it the panel says whose it is
// instead of asking (and drawing a 403) every five seconds.
let canAudit = false;

async function loadBranchCount() {
  // ADR-0005: the branch count gauge, human-readable next to the raw
  // metrics link (the per-branch update counter lives in /metrics).
  try {
    const res = await apiCall("/admin/branches");
    if (res.ok) {
      const branches = /** @type {CatalogBranch[]} */ (await res.json());
      /** @type {HTMLElement} */ (document.getElementById("branch-count-stat")).textContent =
        `${String(branches.length)} sanshain-branch${branches.length === 1 ? "" : "es"}`;
    }
  } catch {
    /* decorative */
  }
}

async function loadStats() {
  try {
    const res = await apiCall("/admin/observability/stats");
    if (res.ok) {
      const data = /** @type {ObservabilityStats} */ (await res.json());
      /** @type {HTMLElement} */ (document.getElementById("stat-cpu")).textContent =
        `${(data.cpu_usage ?? 0).toFixed(1)}%`;
      /** @type {HTMLElement} */ (document.getElementById("stat-mem")).textContent =
        `${((data.memory_used ?? 0) / 1024 / 1024).toFixed(0)} / ${((data.memory_total ?? 0) / 1024 / 1024).toFixed(0)} MB`;
      /** @type {HTMLElement} */ (document.getElementById("stat-requests")).textContent = String(
        data.requests_total ?? 0,
      );
      /** @type {HTMLElement} */ (document.getElementById("stat-failures")).textContent = String(
        data.failures_total ?? 0,
      );
      /** @type {HTMLElement} */ (document.getElementById("stat-uptime")).textContent =
        formatUptime(data.process_uptime ?? 0);
    }
  } catch {
    // The numbers stay as they were until the next refresh.
  }
}

/**
 * @param {number} seconds
 * @returns {string}
 */
function formatUptime(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (h > 0) return `${String(h)}h ${String(m)}m ${String(s)}s`;
  if (m > 0) return `${String(m)}m ${String(s)}s`;
  return `${String(s)}s`;
}

/** @type {ObservabilityDebugConfig} */
let debugConfig = { business_logic_debug: false, admin_user_debug: false };

async function loadDebugConfig() {
  try {
    const res = await apiCall("/admin/observability/debug-config");
    if (res.ok) {
      debugConfig = /** @type {ObservabilityDebugConfig} */ (await res.json());
      updateDebugUI();
    }
  } catch {
    // The switches stay as they were.
  }
}

function updateDebugUI() {
  const logicToggle = /** @type {HTMLElement} */ (document.getElementById("toggle-debug-logic"));
  const logicDot = /** @type {HTMLElement} */ (document.getElementById("dot-debug-logic"));
  const adminToggle = /** @type {HTMLElement} */ (document.getElementById("toggle-debug-admin"));
  const adminDot = /** @type {HTMLElement} */ (document.getElementById("dot-debug-admin"));

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

/** @param {string} type `logic` or `admin`. */
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
  } catch {
    // The switch keeps its old look; the next load shows what the server holds.
  }
}

/** @type {Record<ObservabilityLogLevel, ObservabilityLogEntry[]>} */
let currentLogs = { errors: [], warnings: [], infos: [], debugs: [] };

async function loadLogs() {
  try {
    const res = await apiCall("/admin/observability/logs");
    if (res.ok) {
      const data = /** @type {Partial<Record<ObservabilityLogLevel, ObservabilityLogEntry[]>>} */ (
        await res.json()
      );
      currentLogs.errors = data.errors ?? [];
      currentLogs.warnings = data.warnings ?? [];
      currentLogs.infos = data.infos ?? [];
      currentLogs.debugs = data.debugs ?? [];
      renderLogs();
    }
  } catch {
    // The logs stay as they were until the next refresh.
  }
}

// The logs at the chosen level and above, oldest first: what the view shows
// and what Copy exports.
/**
 * @param {string} filter
 * @returns {ObservabilityLogEntry[]}
 */
function filteredLogs(filter) {
  /** @type {Record<"ALL" | "INFO" | "WARN" | "ERROR", ObservabilityLogLevel[]>} */
  const levels = {
    ALL: ["errors", "warnings", "infos", "debugs"],
    INFO: ["errors", "warnings", "infos"],
    WARN: ["errors", "warnings"],
    ERROR: ["errors"],
  };
  const chosen = /** @type {Record<string, ObservabilityLogLevel[] | undefined>} */ (levels)[
    filter
  ];
  return (chosen ?? levels.ALL)
    .flatMap((level) => currentLogs[level])
    .sort((a, b) => a.timestamp.localeCompare(b.timestamp));
}

function renderLogs() {
  const container = /** @type {HTMLElement} */ (document.getElementById("log-container"));
  const filter = /** @type {HTMLSelectElement} */ (document.getElementById("log-filter")).value;

  const allLogs = filteredLogs(filter);

  if (allLogs.length === 0) {
    container.innerHTML = '<div class="text-slate-500 italic">No logs captured yet.</div>';
    return;
  }

  container.innerHTML = allLogs
    .map((l) => {
      const color =
        /** @type {Record<string, string>} */ ({
          ERROR: "text-red-400",
          WARN: "text-amber-400",
          INFO: "text-indigo-300",
          DEBUG: "text-slate-500",
          TRACE: "text-slate-600",
        })[l.level] ?? "text-slate-300";

      // Some events (provide/require) carry service/version context; show it as
      // a compact chip when present. Most lines (auth, startup, migrations)
      // have none, so the chip is omitted rather than padded with a placeholder.
      const context = l.service
        ? `<span class="text-teal-400">[${escapeHtml(l.service)}${l.version ? "@" + escapeHtml(l.version) : ""}]</span> `
        : "";
      // The reader's own date and time; the module path is a developer's
      // detail and stays on hover (and in the copied text).
      return `<div class="mb-1 log-line" title="${escapeHtml(l.target)}"><span class="text-slate-500 log-time">[${escapeHtml(logLocalTime(l.timestamp))}]</span> <span class="${color} font-bold">${l.level.padEnd(5)}</span> ${context}${escapeHtml(l.message)}</div>`;
    })
    .join("");

  const autoScroll = /** @type {HTMLInputElement | null} */ (
    document.getElementById("log-autoscroll")
  );
  if (autoScroll?.checked) {
    container.scrollTop = container.scrollHeight;
  }
}

// A log instant (RFC 3339, UTC) as the reader's local `YYYY-MM-DD HH:MM:SS`.
// An unparsable value is shown as it came.
/**
 * @param {string} timestamp
 * @returns {string}
 */
function logLocalTime(timestamp) {
  const d = new Date(timestamp);
  if (Number.isNaN(d.getTime())) return timestamp;
  /** @param {number} n */
  const pad = (n) => String(n).padStart(2, "0");
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** @returns {string} */
function getLogsAsText() {
  const filter = /** @type {HTMLSelectElement} */ (document.getElementById("log-filter")).value;
  const allLogs = filteredLogs(filter);
  // Same [service@version] context as the on-screen view: without it,
  // exported provide/require lines name no producer at all.
  return allLogs
    .map((l) => {
      const context = l.service ? `[${l.service}${l.version ? "@" + l.version : ""}] ` : "";
      // Copied lines keep the exact UTC instant and the module: they end up in
      // tickets and chats, read by people in other time zones.
      return `${l.timestamp} ${l.level.padEnd(5)} ${l.target}: ${context}${l.message}`;
    })
    .join("\n");
}

function copyLogs() {
  const text = getLogsAsText();
  if (!text) {
    return;
  }
  // eslint-disable-next-line @typescript-eslint/no-deprecated -- The dispatcher hands this handler no event, so the button is read from the event in progress.
  const clicked = /** @type {Event} */ (event);
  const btn = /** @type {HTMLButtonElement} */ (
    /** @type {Element} */ (clicked.target).closest("button")
  );
  const orig = btn.textContent;
  function onSuccess() {
    btn.textContent = "✓ Copied";
    setTimeout(() => {
      btn.textContent = orig;
    }, 1500);
  }
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- A page served without TLS has no navigator.clipboard, which the DOM types do not say.
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

/**
 * @param {string} text
 * @param {() => void} onSuccess
 */
function fallbackCopy(text, onSuccess) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.left = "-9999px";
  document.body.appendChild(ta);
  ta.select();
  try {
    // eslint-disable-next-line @typescript-eslint/no-deprecated -- This is the fallback for pages without the async clipboard, which only have execCommand.
    document.execCommand("copy");
    onSuccess();
  } catch {
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
  void loadStats();
  void loadLogs();
  void loadAuditLogs();
}, 5000);

// Fetch well past what is shown: rejections share this table, so a CI job
// retrying a breaking change used to push every real change out of a fixed
// 30-row window. Hiding them client-side then still leaves changes to see.
const AUDIT_PANEL_FETCH = 100;
const AUDIT_PANEL_SHOW = 30;

async function loadAuditLogs() {
  if (!canAudit) {
    const body = document.getElementById("audit-log-body");
    if (body && !body.querySelector("[data-locked]")) {
      body.innerHTML =
        '<tr><td colspan="6" class="p-3 text-sm text-slate-400 italic" data-locked>Visible to holders of the view_audit permission.</td></tr>';
    }
    return;
  }
  try {
    const res = await apiCall(`/admin/observability/audit-logs?limit=${String(AUDIT_PANEL_FETCH)}`);
    if (res.ok) {
      const logs = /** @type {ObservabilityAuditEntry[] | null} */ (await res.json());
      renderAuditLogs(logs);
    } else if (res.status === 403) {
      const body = /** @type {HTMLElement} */ (document.getElementById("audit-log-body"));
      body.innerHTML =
        '<tr><td colspan="5" class="p-4 text-center text-slate-400 italic">Admin access required to view the audit log.</td></tr>';
    }
  } catch {
    // The panel stays as it was until the next refresh.
  }
}

/** @param {ObservabilityAuditEntry[] | null} logs */
function renderAuditLogs(logs) {
  const body = /** @type {HTMLElement} */ (document.getElementById("audit-log-body"));
  const hideRejections = /** @type {HTMLInputElement | null} */ (
    document.getElementById("audit-hide-rejections")
  )?.checked;
  const all = logs ?? [];
  const rejections = all.filter((l) => l.action_type === "REJECT").length;
  const shown = (hideRejections ? all.filter((l) => l.action_type !== "REJECT") : all).slice(
    0,
    AUDIT_PANEL_SHOW,
  );
  const note = document.getElementById("audit-rejection-note");
  if (note) {
    note.textContent = rejections
      ? `${String(rejections)} rejection${rejections === 1 ? "" : "s"} in the last ${String(AUDIT_PANEL_FETCH)} entries`
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
                    <td class="p-3 text-slate-400 font-mono">${String(log.id)}</td>
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
    alert("Error exporting audit logs: " + String(e));
  }
}

void checkSession();
