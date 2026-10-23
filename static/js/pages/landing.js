// Extracted from templates/index.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

hideLoader();
renderBanner();
loadDashboard();

// The dashboard: numbers for everyone, the latest provides with names only
// for a signed-in user (the endpoint refuses everyone else).
async function loadDashboard() {
  try {
    const res = await fetch("/api/dashboard");
    if (!res.ok) return;
    const stats = await res.json();
    document.getElementById("stat-producers").textContent = stats.producers;
    document.getElementById("stat-consumers").textContent = stats.consumers;
    document.getElementById("stat-versions").textContent = stats.versions;
    document.getElementById("stat-provides-week").textContent = stats.provides_last_7_days;
    renderActivityChart(stats.provides_per_day);
  } catch (_) {
    // The landing page works without its numbers.
  }
  if (!getSanshainToken()) return;
  try {
    const res = await apiCall("/api/dashboard/recent");
    if (!res.ok) return;
    renderRecentProvides(await res.json());
  } catch (_) {}
}

function renderActivityChart(days) {
  const chart = document.getElementById("activity-chart");
  const max = Math.max(1, ...days.map((d) => d.provides));
  chart.replaceChildren(
    ...days.map((d) => {
      const bar = document.createElement("div");
      bar.className = `flex-1 rounded-t ${d.provides ? "bg-indigo-400" : "bg-slate-100"}`;
      bar.style.height = `${Math.max(4, (d.provides / max) * 100)}%`;
      bar.title = `${d.day}: ${d.provides} provide${d.provides === 1 ? "" : "s"}`;
      bar.dataset.day = d.day;
      bar.dataset.provides = d.provides;
      return bar;
    }),
  );
}

function renderRecentProvides(provides) {
  const list = document.getElementById("recent-provides");
  if (provides.length === 0) {
    list.innerHTML = '<li class="py-2 text-slate-400 italic">Nothing provided yet.</li>';
  } else {
    list.innerHTML = provides
      .map(
        (p) => `<li class="py-2 flex justify-between gap-3">
          <span><a class="text-indigo-700 hover:underline" href="/producers.html?service=${encodeURIComponent(p.producer || "")}">${escapeHtml(p.producer || "")}</a>
            <span class="font-mono text-slate-600">${escapeHtml(p.version || "")}</span>
            ${externalTag(p)}</span>
          <span class="text-xs text-slate-400">${escapeHtml(personLabel(p))} · ${escapeHtml(new Date(p.timestamp).toLocaleString())}</span>
        </li>`,
      )
      .join("");
  }
  document.getElementById("recent-provides-card").classList.remove("hidden");
}

// Lazy format detection: suggest the matching button as the user
// pastes — detection preselects, the click decides.
const validatorInput = document.getElementById("validator-input");
function detectFormat(text) {
  const head = text.slice(0, 2000);
  if (/^\s*asyncapi\s*:/m.test(head)) return "asyncapi";
  if (/^\s*openapi\s*:|^\s*swagger\s*:/m.test(head)) return "openapi";
  if (/^\s*syntax\s*=|^\s*package\s+[\w.]+\s*;|\bservice\s+\w+\s*\{/m.test(head)) return "proto";
  return null;
}
function highlightSuggestion() {
  const detected = detectFormat(validatorInput.value);
  document.querySelectorAll(".validator-btn").forEach((btn) => {
    const isMatch = btn.dataset.validate === detected;
    btn.classList.toggle("ring-2", isMatch);
    btn.classList.toggle("ring-offset-1", isMatch);
    btn.classList.toggle("ring-indigo-400", isMatch);
  });
}
validatorInput.addEventListener("input", highlightSuggestion);
validatorInput.addEventListener("paste", () => setTimeout(highlightSuggestion, 0));

async function runValidation(apiType) {
  const content = validatorInput.value;
  const resultEl = document.getElementById("validator-result");
  const render = (ok, html) => {
    resultEl.classList.remove("hidden");
    resultEl.className =
      "mt-5 rounded-xl border p-4 text-sm text-left " +
      (ok
        ? "border-emerald-200 bg-emerald-50 text-emerald-900"
        : "border-rose-200 bg-rose-50 text-rose-900");
    resultEl.innerHTML = html;
  };
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  if (!content.trim()) {
    render(false, "Paste a specification first.");
    return;
  }
  try {
    const res = await fetch("/validate", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ api_type: apiType, content }),
    });
    if (!res.ok) {
      render(false, `The validator answered ${res.status}.`);
      return;
    }
    const verdict = await res.json();
    if (verdict.valid) {
      const preview = (verdict.endpoints || [])
        .map((e) => `<li class="font-mono text-xs">${esc(e)}</li>`)
        .join("");
      const more =
        verdict.endpoint_count > (verdict.endpoints || []).length
          ? `<p class="text-xs mt-1 opacity-70">… and ${verdict.endpoint_count - verdict.endpoints.length} more</p>`
          : "";
      render(
        true,
        `<p class="font-semibold">✓ Valid ${esc(apiType)} — version ${esc(verdict.version)}, ` +
          `${verdict.endpoint_count} endpoint${verdict.endpoint_count === 1 ? "" : "s"}</p>` +
          (preview
            ? `<p class="mt-2 text-xs font-semibold opacity-70">Sanshain would store:</p><ul class="mt-1 space-y-0.5">${preview}</ul>${more}`
            : ""),
      );
    } else {
      const preview = (verdict.endpoints || [])
        .map((e) => `<li class="font-mono text-xs">${esc(e)}</li>`)
        .join("");
      render(
        false,
        `<p class="font-semibold">✗ Not valid as ${esc(apiType)}</p>` +
          `<p class="mt-1">${esc(verdict.error || "Unknown error")}</p>` +
          (preview
            ? `<p class="mt-2 text-xs font-semibold opacity-70">The document itself splits into:</p><ul class="mt-1 space-y-0.5">${preview}</ul>`
            : ""),
      );
    }
  } catch (e) {
    render(false, "Error: " + esc(e.message));
  }
}
