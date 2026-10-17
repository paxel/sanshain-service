// Webhooks (#47): the hook panel shared by the admin page (global hooks),
// the Producer page and the Consumer page (a system's hooks). One function
// renders the create form, the secret-shown-once box, the hook list and
// each hook's delivery log against a scope's base path
// (`/admin/webhooks`, `/admin/producers/{name}/webhooks`,
// `/admin/consumers/{name}/webhooks`). Everything caller-supplied — URLs,
// errors, event names — is set as text or escaped; controls are bound as
// listeners, never inline.

const WEBHOOK_EVENT_TYPES = [
  "version.published",
  "provide.rejected",
  "version.deprecated",
  "dependency.recorded",
  "branch.cut",
  "catalog.recorded",
];

const WEBHOOK_FORMATS = ["json", "slack", "teams"];

function webhookStatusClass(status) {
  switch (status) {
    case "delivered":
      return "bg-green-100 text-green-700";
    case "failed":
      return "bg-amber-100 text-amber-800";
    case "dead":
      return "bg-red-100 text-red-700";
    default:
      return "bg-slate-100 text-slate-600";
  }
}

function webhookStamp(iso) {
  return (iso || "").slice(0, 19).replace("T", " ");
}

// Confirm through the page's modal when it has one, else the browser's.
function webhookConfirm(message, onConfirm) {
  if (document.getElementById("confirm-modal") && typeof confirmAction === "function") {
    confirmAction(escapeHtml(message), onConfirm);
  } else if (window.confirm(message)) {
    onConfirm();
  }
}

// Render the panel into `container`. Resolves to `true` when the caller may
// see this scope's hooks, `false` (container left empty) when the server
// refuses — a plain user on a Producer page.
async function renderWebhookPanel(container, basePath) {
  container.innerHTML = "";
  let hooks;
  try {
    const res = await apiCall(basePath);
    if (!res.ok) return false;
    hooks = await res.json();
  } catch (_) {
    return false;
  }
  container.setAttribute("data-webhook-panel", basePath);

  // --- Create form ---
  const form = document.createElement("form");
  form.className = "bg-slate-50 rounded-lg p-4 mb-4 space-y-3";
  form.setAttribute("data-webhook-form", "");
  form.innerHTML = `
    <h3 class="text-sm font-medium text-slate-700">Register a hook</h3>
    <div class="flex flex-wrap items-end gap-3">
      <div class="grow min-w-64">
        <label class="block text-xs font-medium text-slate-500 mb-1">Receiver URL</label>
        <input type="url" required data-webhook-url placeholder="https://hooks.slack.com/services/…"
          class="w-full px-3 py-1.5 border border-slate-200 rounded-lg text-sm focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500" />
      </div>
      <div>
        <label class="block text-xs font-medium text-slate-500 mb-1">Format</label>
        <select data-webhook-format class="px-3 py-1.5 border border-slate-200 rounded-lg text-sm">
          ${WEBHOOK_FORMATS.map((f) => `<option value="${f}">${f}</option>`).join("")}
        </select>
      </div>
      <div class="min-w-40">
        <label class="block text-xs font-medium text-slate-500 mb-1">Producer glob (optional)</label>
        <input type="text" data-webhook-glob placeholder="payments-*"
          class="w-full px-3 py-1.5 border border-slate-200 rounded-lg text-sm focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500" />
      </div>
      <button type="submit" data-webhook-create
        class="px-3 py-1.5 bg-indigo-600 text-white text-sm rounded-lg hover:bg-indigo-700 transition-colors">
        Create hook
      </button>
    </div>
    <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-slate-600" data-webhook-types>
      <span class="text-slate-400">Events (none checked = every event):</span>
      ${WEBHOOK_EVENT_TYPES.map(
        (t) =>
          `<label class="inline-flex items-center gap-1"><input type="checkbox" value="${t}" class="rounded-sm" /> ${t}</label>`,
      ).join("")}
    </div>
    <div data-webhook-error class="hidden text-sm text-red-600"></div>
    <div data-webhook-secret-box class="hidden bg-green-50 border border-green-200 p-3 rounded-lg">
      <p class="text-sm font-medium text-green-800 mb-1">Hook created — copy the signing secret now, it will not be shown again.</p>
      <div class="flex items-center gap-2">
        <code data-webhook-secret class="flex-1 bg-white text-green-900 px-3 py-2 rounded-sm border border-green-200 text-xs font-mono break-all select-all"></code>
        <button type="button" data-webhook-copy class="bg-green-600 text-white px-3 py-1.5 rounded-sm text-xs hover:bg-green-700 transition-colors">Copy</button>
      </div>
    </div>`;
  container.appendChild(form);

  const errorEl = form.querySelector("[data-webhook-error]");
  const secretBox = form.querySelector("[data-webhook-secret-box]");
  const secretEl = form.querySelector("[data-webhook-secret]");
  const showError = (message) => {
    errorEl.textContent = message;
    errorEl.classList.remove("hidden");
  };
  const showSecret = (secret, title) => {
    secretBox.querySelector("p").textContent = title;
    secretEl.textContent = secret;
    secretBox.classList.remove("hidden");
  };
  form.querySelector("[data-webhook-copy]").addEventListener("click", () => {
    navigator.clipboard.writeText(secretEl.textContent).catch(() => {});
  });

  const list = document.createElement("div");
  list.className = "space-y-2";
  list.setAttribute("data-webhook-list", "");
  container.appendChild(list);

  const reload = async () => {
    try {
      const res = await apiCall(basePath);
      hooks = res.ok ? await res.json() : [];
    } catch (_) {
      hooks = [];
    }
    renderWebhookList(list, basePath, hooks, { reload, showSecret, showError });
  };

  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    errorEl.classList.add("hidden");
    secretBox.classList.add("hidden");
    const body = {
      url: form.querySelector("[data-webhook-url]").value.trim(),
      format: form.querySelector("[data-webhook-format]").value,
      event_types: [...form.querySelectorAll("[data-webhook-types] input:checked")].map(
        (i) => i.value,
      ),
    };
    const glob = form.querySelector("[data-webhook-glob]").value.trim();
    if (glob) body.producer_glob = glob;
    try {
      const res = await apiCall(basePath, { method: "POST", body });
      if (!res.ok) {
        showError(await errorMessage(res));
        return;
      }
      const created = await res.json();
      form.querySelector("[data-webhook-url]").value = "";
      form.querySelector("[data-webhook-glob]").value = "";
      form.querySelectorAll("[data-webhook-types] input").forEach((i) => {
        i.checked = false;
      });
      showSecret(
        created.secret,
        "Hook created — copy the signing secret now, it will not be shown again.",
      );
      await reload();
    } catch (err) {
      showError(err.message);
    }
  });

  renderWebhookList(list, basePath, hooks, { reload, showSecret, showError });
  return true;
}

function renderWebhookList(list, basePath, hooks, actions) {
  list.innerHTML = "";
  if (!hooks.length) {
    const empty = document.createElement("p");
    empty.className = "text-sm text-slate-500 italic";
    empty.textContent = "No hooks registered yet.";
    list.appendChild(empty);
    return;
  }
  for (const hook of hooks) {
    list.appendChild(renderWebhookRow(basePath, hook, actions));
  }
}

function renderWebhookRow(basePath, hook, actions) {
  const hookPath = `${basePath}/${hook.id}`;
  const row = document.createElement("div");
  row.className = "border border-slate-200 rounded-lg px-3 py-2 text-sm";
  row.setAttribute("data-webhook-row", String(hook.id));
  const filters = [];
  if (hook.event_types && hook.event_types.length) filters.push(hook.event_types.join(", "));
  if (hook.producer_glob) filters.push(`producers ${hook.producer_glob}`);
  row.innerHTML = `
    <div class="flex items-start justify-between gap-3 flex-wrap">
      <div class="min-w-0 grow">
        <div class="flex items-center gap-2 flex-wrap">
          <span class="font-mono text-xs text-slate-800 break-all" data-webhook-row-url></span>
          <span class="px-1.5 py-0.5 rounded-md text-[9px] font-bold uppercase tracking-wider bg-slate-100 text-slate-500 border border-slate-200">${escapeHtml(hook.format)}</span>
          ${hook.paused ? '<span class="px-1.5 py-0.5 rounded-md text-[9px] font-bold uppercase tracking-wider bg-amber-100 text-amber-800" data-webhook-paused>paused</span>' : ""}
          ${hook.allow_list_exempt ? '<span class="px-1.5 py-0.5 rounded-md text-[9px] font-bold uppercase tracking-wider bg-indigo-50 text-indigo-600" title="Registered by an administrator: the allow-list does not apply">exempt</span>' : ""}
        </div>
        <div class="text-xs text-slate-500 mt-1">
          ${filters.length ? escapeHtml(filters.join(" · ")) : "every event"} ·
          secret <span class="font-mono" data-webhook-hint>${escapeHtml(hook.secret_hint)}…</span> ·
          by ${escapeHtml(hook.created_by)} on ${escapeHtml(webhookStamp(hook.created_at))}
        </div>
      </div>
      <div class="flex items-center gap-1 flex-wrap">
        <button type="button" data-act="toggle" class="px-2 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-slate-700">${hook.paused ? "Resume" : "Pause"}</button>
        <button type="button" data-act="test" class="px-2 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-slate-700">Send test</button>
        <button type="button" data-act="rotate" class="px-2 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-slate-700">Rotate secret</button>
        <button type="button" data-act="log" class="px-2 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-indigo-600">Deliveries</button>
        <button type="button" data-act="delete" class="px-2 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-red-50 text-red-600">Delete</button>
      </div>
    </div>
    <div data-webhook-log class="hidden mt-2"></div>`;
  row.querySelector("[data-webhook-row-url]").textContent = hook.url;
  const logEl = row.querySelector("[data-webhook-log]");

  const fail = async (res, what) => {
    actions.showError(`${what}: ${await errorMessage(res)}`);
  };

  row.querySelector('[data-act="toggle"]').addEventListener("click", async () => {
    const res = await apiCall(hookPath, { method: "PATCH", body: { paused: !hook.paused } });
    if (!res.ok) return fail(res, "Could not change the hook");
    actions.reload();
  });
  row.querySelector('[data-act="test"]').addEventListener("click", async () => {
    const res = await apiCall(`${hookPath}/test`, { method: "POST" });
    if (!res.ok) return fail(res, "Could not queue the test event");
    logEl.classList.remove("hidden");
    renderWebhookLog(logEl, hookPath, actions);
  });
  row.querySelector('[data-act="rotate"]').addEventListener("click", () => {
    webhookConfirm(
      "Rotate the signing secret? The receiver must be updated; deliveries from now on are signed with the new one.",
      async () => {
        const res = await apiCall(`${hookPath}/rotate-secret`, { method: "POST" });
        if (!res.ok) return fail(res, "Could not rotate the secret");
        const rotated = await res.json();
        actions.showSecret(
          rotated.secret,
          "Secret rotated — copy the new signing secret now, it will not be shown again.",
        );
        actions.reload();
      },
    );
  });
  row.querySelector('[data-act="log"]').addEventListener("click", () => {
    const open = logEl.classList.toggle("hidden");
    if (!open) renderWebhookLog(logEl, hookPath, actions);
  });
  row.querySelector('[data-act="delete"]').addEventListener("click", () => {
    webhookConfirm(`Delete the hook for ${hook.url}? Its delivery log goes with it.`, async () => {
      const res = await apiCall(hookPath, { method: "DELETE" });
      if (!res.ok) return fail(res, "Could not delete the hook");
      actions.reload();
    });
  });
  return row;
}

async function renderWebhookLog(logEl, hookPath, actions) {
  logEl.innerHTML = '<p class="text-xs text-slate-400">Loading deliveries…</p>';
  let rows;
  try {
    const res = await apiCall(`${hookPath}/deliveries?limit=50`);
    if (!res.ok) {
      logEl.innerHTML = "";
      return actions.showError(`Could not load the deliveries: ${await errorMessage(res)}`);
    }
    rows = await res.json();
  } catch (err) {
    logEl.innerHTML = "";
    return actions.showError(err.message);
  }
  logEl.innerHTML = "";
  if (!rows.length) {
    logEl.innerHTML = '<p class="text-xs text-slate-400 italic">No deliveries yet.</p>';
    return;
  }
  const table = document.createElement("table");
  table.className = "w-full text-xs";
  table.setAttribute("data-webhook-deliveries", "");
  table.innerHTML = `
    <thead><tr class="text-left text-slate-400">
      <th class="py-1 pr-2 font-medium">Event</th><th class="py-1 pr-2 font-medium">Occurred</th>
      <th class="py-1 pr-2 font-medium">Status</th><th class="py-1 pr-2 font-medium">Attempts</th>
      <th class="py-1 pr-2 font-medium">Next attempt</th><th class="py-1 pr-2 font-medium">Response</th>
      <th class="py-1 pr-2 font-medium">Last error</th><th class="py-1"></th>
    </tr></thead>`;
  const tbody = document.createElement("tbody");
  for (const d of rows) {
    const tr = document.createElement("tr");
    tr.className = "border-t border-slate-100 align-top";
    tr.setAttribute("data-webhook-delivery", String(d.id));
    const retrying = d.status === "pending" || d.status === "failed";
    tr.innerHTML = `
      <td class="py-1 pr-2 font-mono">${escapeHtml(d.event_type)}</td>
      <td class="py-1 pr-2 whitespace-nowrap">${escapeHtml(webhookStamp(d.occurred_at))}</td>
      <td class="py-1 pr-2"><span class="px-1.5 py-0.5 rounded-full font-semibold ${webhookStatusClass(d.status)}" data-webhook-status>${escapeHtml(d.status)}</span></td>
      <td class="py-1 pr-2">${Number(d.attempts) || 0}</td>
      <td class="py-1 pr-2 whitespace-nowrap">${retrying ? escapeHtml(webhookStamp(d.next_attempt_at)) : "—"}</td>
      <td class="py-1 pr-2">${d.last_response_code == null ? "—" : escapeHtml(String(d.last_response_code))}</td>
      <td class="py-1 pr-2 text-slate-500 break-all" data-webhook-last-error></td>
      <td class="py-1 text-right"><button type="button" data-act="redeliver" class="px-2 py-0.5 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-indigo-600">Redeliver</button></td>`;
    tr.querySelector("[data-webhook-last-error]").textContent = d.last_error || "";
    tr.querySelector('[data-act="redeliver"]').addEventListener("click", async () => {
      const res = await apiCall(`${hookPath}/deliveries/${d.id}/redeliver`, { method: "POST" });
      if (!res.ok) return actions.showError(`Could not redeliver: ${await errorMessage(res)}`);
      renderWebhookLog(logEl, hookPath, actions);
    });
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  logEl.appendChild(table);
}
