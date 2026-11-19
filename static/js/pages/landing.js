// Extracted from templates/index.html: inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// What the validator buttons name in their data-click attributes.
/* exported runValidation */

/**
 * `DayCount` in `src/application/dashboard_service.rs`.
 * @typedef {object} LandingDay
 * @property {string} day
 * @property {number} provides
 */

/**
 * `DashboardStats` in `src/application/dashboard_service.rs`: `GET /api/dashboard`.
 * @typedef {object} LandingStats
 * @property {number} producers
 * @property {number} consumers
 * @property {number} versions
 * @property {number} provides_last_7_days
 * @property {LandingDay[]} provides_per_day
 */

/**
 * `RecentProvide` in `src/application/dashboard_service.rs`: `GET /api/dashboard/recent`.
 * @typedef {object} LandingProvide
 * @property {string} timestamp
 * @property {string | null} [producer]
 * @property {boolean} [external]
 * @property {string | null} [version]
 * @property {string} username
 * @property {string | null} [display_name]
 */

/**
 * `ValidationReport` in `src/application/spec_service.rs`: the answer of `POST /validate`. The
 * server leaves out what is empty.
 * @typedef {object} LandingVerdict
 * @property {boolean} valid
 * @property {string} [version]
 * @property {number} endpoint_count Read without a guard; present whenever the document is valid.
 * @property {string[]} [endpoints]
 * @property {string} [error]
 */

hideLoader();
void renderBanner();
void loadDashboard();

// The dashboard: numbers for everyone, the latest provides with names only
// for a signed-in user (the endpoint refuses everyone else).
async function loadDashboard() {
  try {
    const res = await fetch("/api/dashboard");
    if (!res.ok) return;
    const stats = /** @type {LandingStats} */ (await res.json());
    /** @type {HTMLElement} */ (document.getElementById("stat-producers")).textContent = String(
      stats.producers,
    );
    /** @type {HTMLElement} */ (document.getElementById("stat-consumers")).textContent = String(
      stats.consumers,
    );
    /** @type {HTMLElement} */ (document.getElementById("stat-versions")).textContent = String(
      stats.versions,
    );
    /** @type {HTMLElement} */ (document.getElementById("stat-provides-week")).textContent = String(
      stats.provides_last_7_days,
    );
    renderActivityChart(stats.provides_per_day);
  } catch {
    // The landing page works without its numbers.
  }
  if (!getSanshainToken()) return;
  try {
    const res = await apiCall("/api/dashboard/recent");
    if (!res.ok) return;
    renderRecentProvides(/** @type {LandingProvide[]} */ (await res.json()));
  } catch {
    // The landing page works without the latest provides.
  }
}

// One bar per day in a 120 x 100 drawing the SVG stretches to the card: the
// heights are attributes of the drawing, not styles of the page.
/** @param {LandingDay[]} days */
function renderActivityChart(days) {
  const chart = /** @type {Element} */ (document.getElementById("activity-chart"));
  const max = Math.max(1, ...days.map((d) => d.provides));
  const slot = 120 / days.length;
  const svg = "http://www.w3.org/2000/svg";
  chart.replaceChildren(
    ...days.map((d, i) => {
      const height = Math.max(4, (d.provides / max) * 100);
      const bar = document.createElementNS(svg, "rect");
      bar.setAttribute("x", String(i * slot + slot * 0.1));
      bar.setAttribute("width", String(slot * 0.8));
      bar.setAttribute("y", String(100 - height));
      bar.setAttribute("height", String(height));
      bar.setAttribute("class", d.provides ? "fill-indigo-400" : "fill-slate-100");
      bar.dataset.day = d.day;
      bar.dataset.provides = String(d.provides);
      const title = document.createElementNS(svg, "title");
      title.textContent = `${d.day}: ${String(d.provides)} provide${d.provides === 1 ? "" : "s"}`;
      bar.appendChild(title);
      return bar;
    }),
  );
}

/** @param {LandingProvide[]} provides */
function renderRecentProvides(provides) {
  const list = /** @type {HTMLElement} */ (document.getElementById("recent-provides"));
  if (provides.length === 0) {
    list.innerHTML = '<li class="py-2 text-slate-400 italic">Nothing provided yet.</li>';
  } else {
    list.innerHTML = provides
      .map(
        (p) => `<li class="py-2 flex justify-between gap-3">
          <span><a class="text-indigo-700 hover:underline" href="/producers.html?service=${encodeURIComponent(p.producer ?? "")}">${escapeHtml(p.producer ?? "")}</a>
            <span class="font-mono text-slate-600">${escapeHtml(p.version ?? "")}</span>
            ${externalTag(p)}</span>
          <span class="text-xs text-slate-400">${escapeHtml(personLabel(p))} · ${escapeHtml(new Date(p.timestamp).toLocaleString())}</span>
        </li>`,
      )
      .join("");
  }
  /** @type {HTMLElement} */ (document.getElementById("recent-provides-card")).classList.remove(
    "hidden",
  );
}

// Lazy format detection: suggest the matching button as the user
// pastes — detection preselects, the click decides.
const validatorInput = /** @type {HTMLTextAreaElement} */ (
  document.getElementById("validator-input")
);
/**
 * @param {string} text
 * @returns {string | null}
 */
function detectFormat(text) {
  const head = text.slice(0, 2000);
  if (/^[ \t]*asyncapi\s*:/m.test(head)) return "asyncapi";
  if (/^[ \t]*openapi\s*:|^[ \t]*swagger\s*:/m.test(head)) return "openapi";
  if (/^[ \t]*syntax\s*=|^[ \t]*package\s+[\w.]+\s*;|\bservice\s+\w+\s*\{/m.test(head))
    return "proto";
  return null;
}
function highlightSuggestion() {
  const detected = detectFormat(validatorInput.value);
  /** @type {NodeListOf<HTMLElement>} */ (document.querySelectorAll(".validator-btn")).forEach(
    (btn) => {
      const isMatch = detected !== null && btn.dataset.validate === detected;
      btn.classList.toggle("ring-2", isMatch);
      btn.classList.toggle("ring-offset-1", isMatch);
      btn.classList.toggle("ring-indigo-400", isMatch);
    },
  );
}
validatorInput.addEventListener("input", highlightSuggestion);
validatorInput.addEventListener("paste", () => setTimeout(highlightSuggestion, 0));

/** @param {string} apiType */
async function runValidation(apiType) {
  const content = validatorInput.value;
  const resultEl = /** @type {HTMLElement} */ (document.getElementById("validator-result"));
  /**
   * @param {boolean} ok
   * @param {string} html
   */
  const render = (ok, html) => {
    resultEl.classList.remove("hidden");
    resultEl.className =
      "mt-5 rounded-xl border p-4 text-sm text-left " +
      (ok
        ? "border-emerald-200 bg-emerald-50 text-emerald-900"
        : "border-rose-200 bg-rose-50 text-rose-900");
    resultEl.innerHTML = html;
  };
  /** @param {unknown} s */
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
      render(false, `The validator answered ${String(res.status)}.`);
      return;
    }
    const verdict = /** @type {LandingVerdict} */ (await res.json());
    if (verdict.valid) {
      const preview = (verdict.endpoints ?? [])
        .map((e) => `<li class="font-mono text-xs">${esc(e)}</li>`)
        .join("");
      const more =
        verdict.endpoint_count > (verdict.endpoints ?? []).length
          ? `<p class="text-xs mt-1 opacity-70">… and ${String(verdict.endpoint_count - /** @type {string[]} */ (verdict.endpoints).length)} more</p>`
          : "";
      render(
        true,
        `<p class="font-semibold">✓ Valid ${esc(apiType)} — version ${esc(verdict.version)}, ` +
          `${String(verdict.endpoint_count)} endpoint${verdict.endpoint_count === 1 ? "" : "s"}</p>` +
          (preview
            ? `<p class="mt-2 text-xs font-semibold opacity-70">Sanshain would store:</p><ul class="mt-1 space-y-0.5">${preview}</ul>${more}`
            : ""),
      );
    } else {
      const preview = (verdict.endpoints ?? [])
        .map((e) => `<li class="font-mono text-xs">${esc(e)}</li>`)
        .join("");
      render(
        false,
        `<p class="font-semibold">✗ Not valid as ${esc(apiType)}</p>` +
          `<p class="mt-1">${esc((verdict.error ?? "") || "Unknown error")}</p>` +
          (preview
            ? `<p class="mt-2 text-xs font-semibold opacity-70">The document itself splits into:</p><ul class="mt-1 space-y-0.5">${preview}</ul>`
            : ""),
      );
    }
  } catch (e) {
    render(false, "Error: " + esc(/** @type {Error} */ (e).message));
  }
}
