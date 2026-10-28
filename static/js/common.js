// Sanshain — shared JS utilities
// Used by admin, account, service, and dashboard pages.
function getCookie(name) {
  const m = document.cookie.match(new RegExp("(?:^|;\\s*)" + name + "=([^;]*)"));
  return m ? m[1] : null;
}

function getSanshainToken() {
  return localStorage.getItem("sanshain_token") || getCookie("sanshain_token");
}

let sanshainToken = getSanshainToken();

function setBannerVersion(version) {
  const text = version ? `v${version}` : "";
  const mobile = document.getElementById("version-badge");
  const desktop = document.getElementById("version-badge-desktop");
  if (mobile) mobile.textContent = text;
  if (desktop) desktop.textContent = text;
}

// --- Permissions ---
//
// The server answers `/auth/me` with the permissions the caller actually holds,
// so the UI asks what someone may *do* rather than whether they are an
// administrator. That is what lets a partial administrator — a user manager, or
// the maintainer of one Producer — see the parts they hold instead of an
// all-or-nothing view.
//
// Hiding a control is a usability affordance, not a security boundary: the route
// guard on the server is the boundary. Never let a hidden button stand in for
// one.
function hasPermission(user, permission) {
  return Array.isArray(user?.permissions) && user.permissions.includes(permission);
}

// True when the caller holds any of the listed permissions.
function hasAnyPermission(user, permissions) {
  return permissions.some((permission) => hasPermission(user, permission));
}

// --- Gated controls ---
//
// A control marked `data-needs="<permission>"` (and, for one Producer's
// actions, `data-needs-producer="<name>"`) is greyed out and inert for a caller
// who may not use it, with a title naming the permission — never hidden, so
// the user learns the feature exists and whom to ask. A maintainer grant on
// the named Producer counts like the global permission, as the server's
// Producer-scoped guard does. Applied to everything already on the page and,
// through an observer, to every row rendered later.
let gateUser = null;
let gateObserver = null;

function gateAllows(user, permission, producer) {
  if (!user) return false;
  if (user.is_root || hasPermission(user, permission)) return true;
  return Boolean(producer) && Array.isArray(user.maintains) && user.maintains.includes(producer);
}

function gateElement(el) {
  const permission = el.dataset.needs;
  if (!permission || !gateUser) return;
  if (gateAllows(gateUser, permission, el.dataset.needsProducer)) {
    if (el.hasAttribute("data-gated")) {
      el.removeAttribute("data-gated");
      el.removeAttribute("aria-disabled");
      el.disabled = false;
      if (el.dataset.gateTitle !== undefined) el.title = el.dataset.gateTitle;
    }
    return;
  }
  if (el.hasAttribute("data-gated")) return;
  el.dataset.gateTitle = el.title || "";
  el.setAttribute("data-gated", "");
  el.setAttribute("aria-disabled", "true");
  if ("disabled" in el) el.disabled = true;
  el.title = `Needs the ${permission} permission${el.dataset.needsProducer ? " or a maintainer grant on " + el.dataset.needsProducer : ""} — ask an administrator`;
}

function applyGates(root = document) {
  if (root.nodeType !== 1 && root !== document) return;
  if (root.matches && root.matches("[data-needs]")) gateElement(root);
  root.querySelectorAll("[data-needs]").forEach(gateElement);
}

// Remember who is signed in and gate the page for them, now and from then on.
function setGateUser(user) {
  gateUser = user;
  applyGates(document);
  if (!gateObserver && typeof MutationObserver !== "undefined" && document.body) {
    gateObserver = new MutationObserver((mutations) => {
      for (const m of mutations) m.addedNodes.forEach((node) => applyGates(node));
    });
    gateObserver.observe(document.body, { childList: true, subtree: true });
  }
}

// Permissions that, between them, mean "there is something on the admin
// dashboard for this person". Kept in one place so a new section does not have
// to remember to extend every caller.
const ADMIN_DASHBOARD_PERMISSIONS = [
  "manage_users",
  "manage_roles",
  "manage_producers",
  "manage_consumers",
  "manage_settings",
  "manage_auth_config",
  "view_audit",
  "view_observability",
  "run_destructive_operations",
];

function canSeeAdminDashboard(user) {
  // A maintainer's permissions are scoped to their Producers rather than
  // granted globally, so `permissions` alone would hide the dashboard from
  // exactly the people expected to manage something there.
  const maintainsSomething = user && Array.isArray(user.maintains) && user.maintains.length > 0;
  return maintainsSomething || hasAnyPermission(user, ADMIN_DASHBOARD_PERMISSIONS);
}

// How a person is named in lists: their display name with the username in
// brackets, or the username alone when they have none.
function personLabel(user) {
  if (!user) return "";
  return user.display_name ? `${user.display_name} (${user.username})` : user.username;
}

// The tag of a Producer the company does not maintain.
function externalTag(producer) {
  return producer && producer.external
    ? '<span class="text-[10px] bg-slate-200 text-slate-600 px-2 py-0.5 rounded-full font-bold uppercase" data-external title="Not maintained by us: an outside API whose spec was provided here">external</span>'
    : "";
}

// A person's avatar: a circle with their initials, and over it their Gravatar
// picture when the instance shows them. Gravatar answers a transparent image
// for an address it does not know, so the initials show through.
function avatarInitials(user) {
  const name = (user.display_name || user.username || "?").trim();
  const words = name.split(/[\s._@-]+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] || "?")[0];
  return letters.toUpperCase();
}
function avatarHtml(user, size = "h-7 w-7 text-xs") {
  const picture = user.gravatar_hash
    ? `<img src="https://www.gravatar.com/avatar/${encodeURIComponent(user.gravatar_hash)}?s=64&d=blank" alt="" class="absolute inset-0 h-full w-full rounded-full" loading="lazy" />`
    : "";
  return `<span class="relative inline-flex ${size} shrink-0 items-center justify-center rounded-full bg-indigo-100 font-semibold text-indigo-700" data-avatar>${escapeHtml(avatarInitials(user))}${picture}</span>`;
}

function updateBannerAuth(user) {
  if (user) setGateUser(user);
  const usernameEl = document.getElementById("banner-username");
  const logoutEl = document.getElementById("banner-logout");
  const signinEl = document.getElementById("banner-signin");
  const adminLink = document.getElementById("nav-admin-link");
  // Each link is shown against the permission its destination actually needs,
  // so a user manager sees the dashboard without being offered the audit
  // timeline they would be refused.
  const auditLink = document.getElementById("nav-audit-link");
  const gatedLinks = [
    [adminLink, () => canSeeAdminDashboard(user)],
    [auditLink, () => hasPermission(user, "view_audit")],
  ];
  if (!usernameEl || !logoutEl || !signinEl) return;

  if (user && user.username) {
    usernameEl.innerHTML = `<span class="inline-flex items-center gap-2">${avatarHtml(user, "h-6 w-6 text-[10px]")}<span>${escapeHtml(user.display_name || user.username)}</span></span>`;
    usernameEl.title = user.username;
    usernameEl.classList.remove("hidden");
    logoutEl.classList.remove("hidden");
    signinEl.classList.add("hidden");
    gatedLinks.forEach(([link, allowed]) => {
      if (link) {
        link.classList.toggle("hidden", !allowed());
      }
    });
  } else {
    usernameEl.textContent = "";
    usernameEl.classList.add("hidden");
    logoutEl.classList.add("hidden");
    signinEl.classList.remove("hidden");
    gatedLinks.forEach(([link]) => {
      if (link) {
        link.classList.add("hidden");
      }
    });
  }
}

function highlightCurrentBannerLink() {
  const nav = document.querySelector("#site-banner nav");
  if (!nav) return;

  const currentPath =
    window.location.pathname === "/" ? "/" : window.location.pathname.replace(/\/+$/, "");
  nav.querySelectorAll("a[href]").forEach((link) => {
    const href = link.getAttribute("href");
    if (!href || !href.startsWith("/")) return;

    const normalizedHref = href === "/" ? "/" : href.replace(/\/+$/, "");
    const isActive = normalizedHref === currentPath;
    link.classList.toggle("font-bold", isActive);
    link.classList.toggle("underline", isActive);
    link.classList.toggle("underline-offset-4", isActive);
    link.classList.toggle("hover:text-indigo-100", !isActive);
  });
}

async function renderBanner(user = null) {
  try {
    highlightCurrentBannerLink();
    if (user && user.username) {
      updateBannerAuth(user);
      return user;
    }

    sanshainToken = getSanshainToken();
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
    initSSEUpdates();
    return data;
  } catch (_) {
    updateBannerAuth(null);
    return null;
  }
}

// Live updates over server-sent events, read with fetch: an EventSource cannot
// send the Authorization header, and a token in the URL was both refused by the
// server and written into every proxy log. A refused stream is not retried —
// it would only repeat the refusal.
async function initSSEUpdates() {
  if (!sanshainToken || window._sseInitialized) return;
  window._sseInitialized = true;
  let retry = true;
  try {
    const res = await fetch("/api/sse/updates", {
      headers: { Authorization: `Bearer ${sanshainToken}`, Accept: "text/event-stream" },
    });
    if (res.status === 401 || res.status === 403) retry = false;
    if (!res.ok || !res.body) return;
    const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
    let buffer = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += value;
      let end;
      while ((end = buffer.indexOf("\n\n")) >= 0) {
        const event = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const updated = event
          .split("\n")
          .some((line) => line.startsWith("data:") && line.slice(5).trim() === "updated");
        if (updated) window.dispatchEvent(new CustomEvent("sanshain-update"));
      }
    }
  } catch (_) {
    // A dropped connection; reconnect below.
  } finally {
    window._sseInitialized = false;
    if (retry) setTimeout(initSSEUpdates, 10000);
  }
}

async function sanshainLogout(options = {}) {
  const redirectTo = Object.prototype.hasOwnProperty.call(options, "redirectTo")
    ? options.redirectTo
    : "/";
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
        html.dark .fill-slate-100 { fill: #334155 !important; }
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

// Reads the message out of a failed response.
//
// Errors are JSON — `{ "error": "..." }`, sometimes with extra fields — so the
// raw body is not fit to show a user. Falls back to the raw text for anything
// that is not the expected shape, such as a proxy or gateway error that never
// reached the service.
async function errorMessage(res) {
  const raw = await res.text();
  if (!raw) return `Request failed (${res.status})`;
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.error === "string") return parsed.error;
  } catch (_) {}
  return raw;
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
  // Refresh token from localStorage/cookie in case it was updated by another script/context
  sanshainToken = getSanshainToken();

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
// Quotes are escaped too: the value is routinely interpolated into a quoted
// attribute (title="…", value="…"), and textContent→innerHTML alone leaves `"`
// intact, which lets a crafted name close the attribute and add its own.
// Harmless in text position — a browser renders &quot; as ".
function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML.replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

// For a value embedded as a single-quoted JS string that itself sits inside a
// quoted HTML attribute. The browser HTML-decodes the attribute before parsing
// the JS, so both layers must be escaped, innermost first: without the HTML
// pass a value containing a double quote closes the attribute and can inject
// markup. Producer, role and branch names are all caller-chosen and
// unrestricted, so they reach here hostile. For a plain attribute value with
// no JS around it, use escapeHtml — it leaves no backslashes behind. For the
// dispatcher's data-<event>-args, use attrJson.
function escapeAttr(str) {
  return String(str)
    .replace(/\\/g, "\\\\")
    .replace(/'/g, "\\'")
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// Encode a value as JSON safe to drop into a double-quoted HTML attribute for
// the delegated dispatcher's data-<event>-args. Unlike escapeAttr this does
// *only* HTML-entity escaping (no backslash/quote JS-escaping), so the string
// getAttribute hands back is byte-for-byte the JSON.stringify output and
// JSON.parse round-trips it — backslash escapes such as \n survive intact.
function attrJson(value) {
  return JSON.stringify(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// The same args, for an element built through the DOM rather than an innerHTML
// string. setAttribute stores its value verbatim — nothing HTML-decodes it — so
// attrJson's entities would survive into the attribute and JSON.parse would
// reject them, leaving the handler to run with no arguments at all. Use this
// whenever the element is created with createElement; use attrJson only inside
// a template literal the browser will parse.
function setActionArgs(el, type, values) {
  el.setAttribute(`data-${type}-args`, JSON.stringify(values));
}

// --- Confirm modal ---
// Requires a #confirm-modal, #confirm-message, #confirm-yes in the page.
function confirmAction(message, onConfirm, title = "Confirm Action", buttonText = "Confirm") {
  const modal = document.getElementById("confirm-modal");
  if (!modal) return;
  const titleEl = modal.querySelector("h3");
  if (titleEl) titleEl.textContent = title;
  const yesBtn = document.getElementById("confirm-yes");
  if (yesBtn) {
    yesBtn.textContent = buttonText;
    yesBtn.onclick = () => {
      closeConfirmModal();
      onConfirm();
    };
  }
  document.getElementById("confirm-message").innerHTML = message;
  modal.classList.remove("hidden");
}
// Alias used by admin page
const confirmDelete = (message, onConfirm) =>
  confirmAction(message, onConfirm, "Confirm Deletion", "Delete");

function closeConfirmModal() {
  document.getElementById("confirm-modal").classList.add("hidden");
}

// --- One-click promote (#28) ---
// Releases a stored snapshot in place through the same GA gate as any release.
// Shared by the producers page and the admin dashboard; `onSuccess` is the
// page's own refresh hook.
function promoteVersion(serviceName, apiType, version, onSuccess) {
  confirmAction(
    `Release ${escapeHtml(apiType)} version <strong>${escapeHtml(version)}</strong> of <strong>${escapeHtml(serviceName)}</strong> as GA?<br><br>The number is permanently claimed and its content becomes immutable.`,
    async () => {
      try {
        const res = await apiCall(
          `/admin/producers/${encodeURIComponent(serviceName)}/versions/${encodeURIComponent(apiType)}/${encodeURIComponent(version)}/promote`,
          { method: "POST" },
        );
        if (!res.ok) {
          alert("Failed to promote: " + (await errorMessage(res)));
          return;
        }
        if (onSuccess) await onSuccess();
      } catch (e) {
        alert("Failed to promote: " + e.message);
      }
    },
    "Promote to GA",
    "Promote",
  );
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
        <button data-click="reloadClearingInstance" style="background:#f59e0b;color:white;border:none;padding:5px 14px;border-radius:6px;cursor:pointer;font-weight:600;font-size:13px;">Reload</button>
        <button data-click="dismissReloadBanner" data-click-args='["$this"]' style="background:none;border:none;cursor:pointer;color:#92400e;font-size:18px;line-height:1;padding:0 4px;" title="Dismiss">&times;</button>
    `;
  document.body.prepend(banner);
}

// Reload / dismiss actions for the staleness banner (were inline handlers).
function reloadClearingInstance() {
  sessionStorage.removeItem("sanshain_instance");
  location.reload(true);
}
function dismissReloadBanner(el) {
  el.parentElement.remove();
  sessionStorage.removeItem("sanshain_instance");
}

// Run staleness check on every page load
sanshainToken = getSanshainToken();
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
    if (window.SANSHAIN_FAST_SCREENSHOT) {
      l.style.display = "none";
    }
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

// ── Delegated event dispatcher ──────────────────
// Replaces inline event-handler attributes so the CSP can forbid inline
// script. A control declares data-<event>="fnName" (plus an optional
// data-<event>-args JSON array) in place of an inline handler attribute; one
// delegated listener per event type resolves the nearest ancestor carrying
// the attribute and calls the named global with the decoded args. Because it
// is delegated on document, it covers markup rendered later by JS exactly as
// it covers static markup.
//
// Arg tokens: "$this" is the element, "$event" the event, "$value" the
// element's current value. Everything else is passed literally. Dynamic
// markup builds the args attribute with attrJson() so the JSON round-trips
// through getAttribute intact.
(function () {
  const EVENTS = ["click", "change", "input", "submit", "keydown"];

  function decodeArgs(el, event, raw) {
    if (!raw) return [];
    let parsed;
    try {
      parsed = JSON.parse(raw);
    } catch (e) {
      console.error("data-action args are not valid JSON:", raw, e);
      return [];
    }
    return parsed.map((a) => {
      if (a === "$this") return el;
      if (a === "$event") return event;
      if (a === "$value") return el.value;
      return a;
    });
  }

  function dispatch(type, event) {
    const el = event.target.closest(`[data-${type}]`);
    if (!el) return;
    // A data-submit control is always an in-page handler; keep the browser
    // from navigating away on native form submission (the old handlers all
    // ended in `return false`).
    if (type === "submit") event.preventDefault();
    const name = el.dataset[type];
    const fn = window[name];
    if (typeof fn !== "function") {
      // A dead action is a real bug once inline handlers are gone; surface it.
      console.error(`data-${type}="${name}" resolves to no function`);
      return;
    }
    const args = decodeArgs(el, event, el.getAttribute(`data-${type}-args`));
    fn.apply(el, args);
  }

  for (const type of EVENTS) {
    document.addEventListener(type, (event) => dispatch(type, event));
  }
})();

// The context block (#48) a system's Backstage catalog-info gives it: owner,
// description, then one row of link chips — the repository first (labelled
// by the host of the source location), then the Backstage links by title.
// Every string is caller-supplied and reaches here hostile, so all of it is
// escaped; chips open in a new tab with rel="noopener noreferrer". `dark`
// picks the palette of the graph's node panel. Empty for a system without a
// context, or one whose context has nothing to show.
function systemContextHtml(context, dark) {
  if (!context) return "";
  const text = dark ? "text-slate-300" : "text-slate-700";
  const muted = dark ? "text-slate-400" : "text-slate-500";
  const chip = dark
    ? "bg-slate-800 text-slate-200 border border-slate-500 hover:bg-slate-700"
    : "bg-slate-50 text-slate-700 border border-slate-200 hover:bg-slate-200";
  const rows = [];
  if (context.owner) {
    rows.push(
      `<div class="text-xs ${text}" data-context="owner"><span class="${muted}">Owner</span> <span class="font-semibold">${escapeHtml(context.owner.name)}</span> <span class="${muted}">(${escapeHtml(context.owner.kind)})</span></div>`,
    );
  }
  if (context.description) {
    rows.push(
      `<p class="text-xs ${text} break-words" data-context="description">${escapeHtml(context.description)}</p>`,
    );
  }
  const chips = [];
  if (context.source_location) {
    let host = context.source_location;
    try {
      host = new URL(context.source_location).host || host;
    } catch (_) {
      /* validated server-side as http(s); the URL itself is a fine label */
    }
    chips.push(linkChip(context.source_location, host, "repository", chip));
  }
  for (const link of context.links || []) {
    chips.push(linkChip(link.url, link.title, "link", chip));
  }
  if (chips.length) {
    rows.push(
      `<div class="flex flex-wrap items-center gap-1.5" data-context="links">${chips.join("")}</div>`,
    );
  }
  if (context.tags && context.tags.length) {
    const tagChip = dark
      ? "bg-indigo-900/60 text-indigo-200 border border-indigo-700"
      : "bg-indigo-50 text-indigo-700 border border-indigo-200";
    rows.push(
      `<div class="flex flex-wrap items-center gap-1" data-context="tags">${context.tags
        .map(
          (tag) =>
            `<span class="px-1.5 py-0.5 rounded-full text-[10px] font-mono ${tagChip}" data-tag="${escapeHtml(tag)}">${escapeHtml(tag)}</span>`,
        )
        .join("")}</div>`,
    );
  }
  return rows.length ? `<div class="space-y-1" data-context>${rows.join("")}</div>` : "";
}

// The ticket URL template of the instance (GET /admin/settings/ticket-url-template),
// fetched once per page; `null` renders ticket ids as plain text.
let _ticketUrlTemplate = null;
let _ticketUrlTemplateLoaded = null;
async function loadTicketUrlTemplate() {
  if (!_ticketUrlTemplateLoaded) {
    _ticketUrlTemplateLoaded = (async () => {
      try {
        const res = await apiCall("/admin/ticket-url-template");
        if (res.ok) _ticketUrlTemplate = (await res.json()).template || null;
      } catch (_) {
        /* ids stay plain text */
      }
    })();
  }
  await _ticketUrlTemplateLoaded;
  return _ticketUrlTemplate;
}

function ticketUrl(template, id) {
  return template ? template.replaceAll("{id}", encodeURIComponent(id)) : null;
}

// A version's origin (what the providing build said about why): the note,
// then chips for the build, the change and each ticket. Empty string when
// the build said nothing.
function originHtml(origin, template, dark) {
  if (!origin) return "";
  const text = dark ? "text-slate-300" : "text-slate-600";
  const chip = dark
    ? "bg-slate-800 text-slate-200 border border-slate-500 hover:bg-slate-700"
    : "bg-slate-50 text-slate-700 border border-slate-200 hover:bg-slate-200";
  const plain = dark
    ? "bg-slate-800 text-slate-300 border border-slate-600"
    : "bg-slate-50 text-slate-600 border border-slate-200";
  const rows = [];
  if (origin.note) {
    rows.push(
      `<p class="text-xs ${text} break-words" data-origin="note">${escapeHtml(origin.note)}</p>`,
    );
  }
  const chips = [];
  // Only the trunk build's link: branch builds are deleted after the merge.
  if (origin.build_url && origin.trunk_build) {
    chips.push(linkChip(origin.build_url, "build", "build", chip));
  }
  if (origin.change_url) chips.push(linkChip(origin.change_url, "change", "change", chip));
  for (const id of origin.tickets || []) {
    const url = ticketUrl(template, id);
    chips.push(
      url
        ? linkChip(url, id, "ticket", chip)
        : `<span data-origin-ticket class="px-2 py-0.5 rounded-full text-[11px] ${plain}">${escapeHtml(id)}</span>`,
    );
  }
  if (chips.length) {
    rows.push(
      `<div class="flex flex-wrap items-center gap-1.5" data-origin="links">${chips.join("")}</div>`,
    );
  }
  return rows.length ? `<div class="space-y-1 mt-1.5" data-origin>${rows.join("")}</div>` : "";
}

function linkChip(url, label, kind, chipClass) {
  return `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer" data-context-link="${kind}" class="px-2 py-0.5 rounded-full text-[11px] ${chipClass}">${escapeHtml(label)}</a>`;
}
