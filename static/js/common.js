// Sanshain — shared JS utilities
// Used by admin, account, service, and dashboard pages.
let sanshainToken = localStorage.getItem("sanshain_token");

function setBannerVersion(version) {
  const text = version ? `v${version}` : "";
  const mobile = document.getElementById("version-badge");
  const desktop = document.getElementById("version-badge-desktop");
  if (mobile) mobile.textContent = text;
  if (desktop) desktop.textContent = text;
}

function updateBannerAuth(user) {
  const usernameEl = document.getElementById("banner-username");
  const logoutEl = document.getElementById("banner-logout");
  const signinEl = document.getElementById("banner-signin");
  if (!usernameEl || !logoutEl || !signinEl) return;

  if (user && user.username) {
    usernameEl.textContent = user.username;
    usernameEl.classList.remove("hidden");
    logoutEl.classList.remove("hidden");
    signinEl.classList.add("hidden");
  } else {
    usernameEl.textContent = "";
    usernameEl.classList.add("hidden");
    logoutEl.classList.add("hidden");
    signinEl.classList.remove("hidden");
  }
}

async function renderBanner(user = null) {
  try {
    if (user && user.username) {
      updateBannerAuth(user);
      return user;
    }

    if (!sanshainToken) {
      updateBannerAuth(null);
      return null;
    }

    const res = await apiCall("/auth/me");
    if (!res.ok) {
      updateBannerAuth(null);
      return null;
    }

    const data = await res.json();
    updateBannerAuth(data);
    return data;
  } catch (_) {
    updateBannerAuth(null);
    return null;
  }
}

async function sanshainLogout(options = {}) {
  const redirectTo = Object.prototype.hasOwnProperty.call(options, "redirectTo")
    ? options.redirectTo
    : "/index.html";
  try {
    if (!csrfToken) {
      await fetchCsrfToken();
    }
    await apiCall("/auth/logout", { method: "POST" });
  } catch (_) {
    // Best effort logout; still clear local session state.
  }

  sanshainToken = null;
  localStorage.removeItem("sanshain_token");
  updateBannerAuth(null);

  if (redirectTo) {
    window.location.href = redirectTo;
  }
}

// --- Dark / Light theme ---
(function () {
  // Read preference from cookie, default to 'light'
  function getThemeCookie() {
    const m = document.cookie.match(/(?:^|;\s*)sanshain_theme=(\w+)/);
    return m ? m[1] : null;
  }
  function setThemeCookie(theme) {
    document.cookie = `sanshain_theme=${theme};path=/;max-age=${365 * 24 * 3600};SameSite=Lax`;
  }
  function applyTheme(theme) {
    const html = document.documentElement;
    if (theme === "dark") {
      html.classList.add("dark");
    } else {
      html.classList.remove("dark");
    }
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", () => applySockeGimmick(theme));
    } else {
      applySockeGimmick(theme);
    }
  }

  function applySockeGimmick(theme) {
    const isDark = theme === "dark";
    const fromText = isDark ? /Sanshain/g : /SOKA/g;
    const toText = isDark ? "SOKA" : "Sanshain";
    const fromJa = isDark ? /サンシャイン/g : /そうか/g;
    const toJa = isDark ? "そうか" : "サンシャイン";
    const fromImg = isDark ? "sonne.png" : "socke.png";
    const toImg = isDark ? "socke.png" : "sonne.png";

    const updateText = (txt) => {
      return txt.replace(fromText, toText).replace(fromJa, toJa);
    };

    // 1. Update <title>
    if (
      document.title.includes(isDark ? "Sanshain" : "SOKA") ||
      document.title.includes(isDark ? "サンシャイン" : "そうか")
    ) {
      document.title = updateText(document.title);
    }

    // 2. Update all text nodes (best effort, limited to headers/nav/footer)
    const selectors = "h1, h2, h3, a, span, div, footer, button, label, p, td, th, li";
    document.querySelectorAll(selectors).forEach((el) => {
      const hasMatch =
        el.textContent.includes(isDark ? "Sanshain" : "SOKA") ||
        el.textContent.includes(isDark ? "サンシャイン" : "そうか");
      if (hasMatch) {
        if (el.children.length === 0) {
          el.textContent = updateText(el.textContent);
        } else if (el.childNodes.length > 0) {
          // Check immediate text nodes
          for (const node of el.childNodes) {
            if (node.nodeType === Node.TEXT_NODE) {
              node.textContent = updateText(node.textContent);
            }
          }
        }
      }
    });

    // 3. Update images
    document.querySelectorAll("img").forEach((img) => {
      if (img.src.includes(fromImg)) {
        img.src = img.src.replace(fromImg, toImg);
        if (
          img.alt.includes(isDark ? "Sanshain" : "SOKA") ||
          img.alt.includes(isDark ? "サンシャイン" : "そうか")
        ) {
          img.alt = updateText(img.alt);
        }
      }
    });
  }
  const saved = getThemeCookie() || "light";
  applyTheme(saved);

  // Inject global dark-mode CSS overrides
  const style = document.createElement("style");
  style.textContent = `
        html.dark body { background: #0f172a !important; color: #e2e8f0 !important; }
        html.dark nav { background: transparent !important; }
        html.dark .bg-white { background: #1e293b !important; }
        html.dark .bg-slate-50 { background: #1e293b !important; }
        html.dark .bg-slate-100 { background: #334155 !important; }
        html.dark .bg-slate-200 { background: #475569 !important; }
        html.dark .text-slate-900 { color: #e2e8f0 !important; }
        html.dark .text-slate-800 { color: #cbd5e1 !important; }
        html.dark .text-slate-700 { color: #cbd5e1 !important; }
        html.dark .text-slate-600 { color: #94a3b8 !important; }
        html.dark .text-slate-500 { color: #94a3b8 !important; }
        html.dark .border-slate-200 { border-color: #475569 !important; }
        html.dark .border-slate-300 { border-color: #475569 !important; }
        html.dark input, html.dark select, html.dark textarea {
            background: #1e293b !important; color: #e2e8f0 !important; border-color: #475569 !important;
        }
        html.dark .bg-slate-800 { background: #0f172a !important; }
        html.dark pre { color: #e2e8f0 !important; }
        html.dark .shadow-lg { box-shadow: 0 10px 15px -3px rgb(0 0 0 / 0.3) !important; }
        html.dark .shadow-md { box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.3) !important; }
        html.dark .divide-slate-200 > :not(:first-child) { border-color: #475569 !important; }
        html.dark code { background: #334155 !important; color: #e2e8f0 !important; }
        html.dark .bg-amber-100 { background: #78350f !important; }
        html.dark .text-amber-700 { color: #fbbf24 !important; }
        html.dark .bg-green-100 { background: #064e3b !important; }
        html.dark .text-green-700 { color: #6ee7b7 !important; }
        html.dark .bg-red-100 { background: #7f1d1d !important; }
        html.dark .text-red-700 { color: #fca5a5 !important; }
        html.dark .bg-blue-100 { background: #1e3a5f !important; }
        html.dark .text-blue-700 { color: #93c5fd !important; }
        html.dark .bg-indigo-50 { background: #312e81 !important; }
        html.dark .text-indigo-700 { color: #a5b4fc !important; }
        html.dark .bg-yellow-50 { background: #422006 !important; }
        html.dark .text-yellow-700 { color: #fde047 !important; }
        html.dark table th { background: #334155 !important; color: #e2e8f0 !important; }
        html.dark table td { border-color: #475569 !important; }
        html.dark .endpoint-card:hover { box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.4) !important; }
        /* Modal overlay */
        html.dark [class*="bg-black/"] { background: rgba(0,0,0,0.7) !important; }
        /* Diff pre blocks — ensure text is always light on dark bg */
        .diff-pre { background: #1e293b !important; color: #e2e8f0 !important; }
        /* Graph filter toolbar row — bg-slate-100/50 is not matched by .bg-slate-100 */
        html.dark #graph-toolbar-row { background: rgba(51,65,85,0.5) !important; }
        html.dark .bg-indigo-100 { background: #312e81 !important; }
        html.dark .text-indigo-400 { color: #818cf8 !important; }
        html.dark .hover\:bg-slate-50:hover { background: #334155 !important; }
        html.dark .bg-red-50 { background: #450a0a !important; }

        /* Cat Loader */
        #app-loader {
            position: fixed; inset: 0; background: rgba(248, 250, 252, 0.9);
            display: flex; flex-direction: column; align-items: center; justify-content: center;
            z-index: 9999; transition: opacity 0.3s ease-out;
        }
        html.dark #app-loader { background: rgba(15, 23, 42, 0.9); }
        #app-loader.hidden { opacity: 0; pointer-events: none; }
        .cat-container { width: 120px; height: 80px; position: relative; }
        .cat-svg { width: 100%; height: 100%; fill: #94a3b8; animation: breathe 3s ease-in-out infinite; }
        @keyframes breathe {
            0%, 100% { transform: scale(1); }
            50% { transform: scale(1.05); }
        }
        .zzz-container { position: absolute; top: 0; right: 10px; font-weight: bold; color: #64748b; font-family: monospace; }
        html.dark .zzz-container { color: #94a3b8 !important; }
        html.dark .cat-svg path[stroke="#1e293b"] { stroke: #e2e8f0 !important; }
        html.dark .cat-svg circle[fill="#1e293b"] { fill: #e2e8f0 !important; }
        .zzz { position: absolute; opacity: 0; animation: floatZ 3s infinite; }
        .zzz:nth-child(1) { animation-delay: 0s; font-size: 14px; }
        .zzz:nth-child(2) { animation-delay: 1s; font-size: 18px; }
        .zzz:nth-child(3) { animation-delay: 2s; font-size: 22px; }
        @keyframes floatZ {
            0% { opacity: 0; transform: translate(0, 0); }
            20% { opacity: 1; }
            80% { opacity: 0; transform: translate(20px, -40px); }
            100% { opacity: 0; }
        }
    `;
  document.head.appendChild(style);

  // Expose toggle function globally
  window.sanshainToggleTheme = function () {
    const current = getThemeCookie() || "light";
    const next = current === "dark" ? "light" : "dark";
    setThemeCookie(next);
    applyTheme(next);
    // Update toggle button icons if present
    document.querySelectorAll(".theme-toggle-icon").forEach((el) => {
      el.textContent = next === "dark" ? "☀️" : "🌙";
    });
  };
  // After DOM ready, inject toggle button into nav
  document.addEventListener("DOMContentLoaded", () => {
    const nav =
      document.querySelector("#site-banner > .container") ||
      document.querySelector("nav .container");
    if (!nav) return;
    const btn = document.createElement("button");
    btn.className =
      "theme-toggle-btn ml-3 px-2 py-1 rounded-lg text-lg hover:bg-indigo-600 transition-colors";
    btn.title = "Toggle dark/light mode";
    btn.innerHTML = `<span class="theme-toggle-icon">${(getThemeCookie() || "light") === "dark" ? "☀️" : "🌙"}</span>`;
    btn.onclick = window.sanshainToggleTheme;
    // Find the right-side flex container in nav
    const rightSide =
      document.querySelector("#site-banner .justify-end") ||
      nav.querySelector(".flex.items-center.space-x-4:last-child") ||
      nav.querySelector(".flex.items-center:last-child") ||
      nav.lastElementChild;
    if (rightSide && rightSide !== nav.firstElementChild) {
      rightSide.prepend(btn);
    } else {
      nav.appendChild(btn);
    }
  });
})();
let csrfToken = null;

// --- Network error helper ---
function friendlyError(err) {
  if (
    err instanceof TypeError &&
    (err.message.includes("NetworkError") ||
      err.message.includes("Failed to fetch") ||
      err.message.includes("Load failed"))
  ) {
    return "Server is not reachable. Please check that the service is running.";
  }
  return err.message || String(err);
}

// --- CSRF ---
async function fetchCsrfToken() {
  try {
    const res = await fetch("/csrf-token");
    if (res.ok) {
      const data = await res.json();
      csrfToken = data.csrf_token;
    }
  } catch (_) {}
}

// --- Authenticated API helper ---
// Automatically attaches Bearer token and CSRF token.
// If body is an object, serialises as JSON.
// On 401, clears session and calls onSessionExpired() if defined.
async function apiCall(url, options = {}) {
  const headers = { ...options.headers };
  if (sanshainToken) headers["Authorization"] = `Bearer ${sanshainToken}`;
  if (options.body && typeof options.body === "object") {
    headers["Content-Type"] = "application/json";
    options.body = JSON.stringify(options.body);
  }
  const method = (options.method || "GET").toUpperCase();
  if (["POST", "PUT", "DELETE", "PATCH"].includes(method) && csrfToken) {
    headers["X-CSRF-Token"] = csrfToken;
  }
  const res = await fetch(url, { ...options, headers });
  if (res.status === 401) {
    sanshainToken = null;
    localStorage.removeItem("sanshain_token");
    if (typeof onSessionExpired === "function") onSessionExpired();
    throw new Error("Session expired");
  }
  return res;
}

// --- HTML / attribute escaping ---
function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

function escapeAttr(str) {
  return str.replace(/\\/g, "\\\\").replace(/'/g, "\\'");
}

// --- Confirm modal ---
// Requires a #confirm-modal, #confirm-message, #confirm-yes in the page.
function confirmAction(message, onConfirm) {
  document.getElementById("confirm-message").innerHTML = message;
  document.getElementById("confirm-modal").classList.remove("hidden");
  document.getElementById("confirm-yes").onclick = () => {
    closeConfirmModal();
    onConfirm();
  };
}
// Alias used by admin page
const confirmDelete = confirmAction;

function closeConfirmModal() {
  document.getElementById("confirm-modal").classList.add("hidden");
}

// --- Version badge ---
function loadVersionBadge(elementId) {
  fetch("/version")
    .then((r) => r.json())
    .then((data) => {
      const el = document.getElementById(elementId);
      if (el) el.textContent = `v${data.version}`;
      setBannerVersion(data.version);
    })
    .catch(() => {});
}

// --- Staleness detection ---
// Checks the server's version + instance_id against what was stored in sessionStorage.
// If they differ (server restarted or updated), shows a reload banner at the top of the page.
function checkStaleness() {
  fetch("/version")
    .then((r) => r.json())
    .then((data) => {
      const key = `${data.version}::${data.instance_id}`;
      const stored = sessionStorage.getItem("sanshain_instance");
      if (!stored) {
        // First visit this session — store and move on
        sessionStorage.setItem("sanshain_instance", key);
        return;
      }
      if (stored !== key) {
        showReloadBanner();
      }
    })
    .catch(() => {});
}

function showReloadBanner() {
  if (document.getElementById("sanshain-reload-banner")) return;
  const banner = document.createElement("div");
  banner.id = "sanshain-reload-banner";
  banner.style.cssText =
    "position:fixed;top:0;left:0;right:0;z-index:9999;background:#fef3c7;border-bottom:2px solid #f59e0b;padding:10px 16px;display:flex;align-items:center;justify-content:center;gap:12px;font-size:14px;color:#92400e;font-family:ui-sans-serif,system-ui,sans-serif;";
  banner.innerHTML = `
        <svg style="width:20px;height:20px;flex-shrink:0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M12 2a10 10 0 100 20 10 10 0 000-20z"/>
        </svg>
        <span>The server has been updated or restarted. You may be viewing stale data.</span>
        <button onclick="sessionStorage.setItem('sanshain_instance','');location.reload()" style="background:#f59e0b;color:white;border:none;padding:5px 14px;border-radius:6px;cursor:pointer;font-weight:600;font-size:13px;">Reload</button>
        <button onclick="this.parentElement.remove();sessionStorage.setItem('sanshain_instance','')" style="background:none;border:none;cursor:pointer;color:#92400e;font-size:18px;line-height:1;padding:0 4px;" title="Dismiss">&times;</button>
    `;
  document.body.prepend(banner);
}

// Run staleness check on every page load
checkStaleness();
loadVersionBadge("version-badge");

// --- Cat Loader ---
function injectLoader() {
  if (document.getElementById("app-loader")) return;
  const loader = document.createElement("div");
  loader.id = "app-loader";
  loader.innerHTML = `
        <div class="cat-container">
            <svg class="cat-svg" viewBox="0 0 100 60">
                <!-- Curled cat body -->
                <path d="M20,50 Q20,20 50,20 Q80,20 80,50 L20,50 Z" />
                <!-- Tail -->
                <path d="M80,50 C95,50 95,30 85,30" fill="none" stroke="#94a3b8" stroke-width="6" stroke-linecap="round" />
                <!-- Ears -->
                <path d="M25,25 L20,10 L35,22 Z" />
                <path d="M45,22 L60,10 L55,25 Z" />
                <!-- Closed eyes -->
                <path d="M28,35 L36,35" stroke="#1e293b" stroke-width="1.5" stroke-linecap="round" />
                <path d="M44,35 L52,35" stroke="#1e293b" stroke-width="1.5" stroke-linecap="round" />
                <!-- Nose -->
                <circle cx="40" cy="40" r="1.5" fill="#1e293b" />
            </svg>
            <div class="zzz-container">
                <span class="zzz">z</span>
                <span class="zzz">z</span>
                <span class="zzz">z</span>
            </div>
        </div>
        <div class="mt-4 text-slate-500 text-sm font-medium tracking-wide">Loading...</div>
    `;
  document.body.appendChild(loader);
}

window.showLoader = function () {
  injectLoader();
  const l = document.getElementById("app-loader");
  l.classList.remove("hidden");
  l.style.opacity = "1";
};

window.hideLoader = function () {
  const l = document.getElementById("app-loader");
  if (l) {
    l.classList.add("hidden");
    l.style.opacity = "0";
  }
};

// --- Password visibility toggle ---
function togglePasswordVisibility(inputId, btn) {
  const input = document.getElementById(inputId);
  if (!input) return;
  if (input.type === "password") {
    input.type = "text";
    btn.textContent = "Hide";
  } else {
    input.type = "password";
    btn.textContent = "Show";
  }
}
