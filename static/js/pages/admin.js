// Extracted from static/admin.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// Use sanshainToken and renderBanner from common.js
// The signed-in caller, for gating affordances (the Promote button).
// Hiding is a usability affordance; the server's GA gate is the boundary.
let adminActor = null;

function onSessionExpired() {
  window.location.href = "/account.html";
}

function showNotAdmin() {
  hideLoader();
  document.getElementById("not-admin-screen").classList.remove("hidden");
  document.getElementById("admin-dashboard").classList.add("hidden");
}

function showDashboard() {
  hideLoader();
  document.getElementById("not-admin-screen").classList.add("hidden");
  document.getElementById("admin-dashboard").classList.remove("hidden");
  document.getElementById("create-branch-btn").addEventListener("click", createBranch);
  loadAll();
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
      adminActor = data;
      // Anyone holding at least one administrative permission has
      // something to do here; the individual sections gate
      // themselves.
      if (!canSeeAdminDashboard(data)) {
        await renderBanner(data);
        showNotAdmin();
        return;
      }
      await renderBanner(data);
      showDashboard();
      // Providing from the browser is a Producer-administration
      // act; the form is shown to those who may do it.
      if (hasPermission(data, "manage_producers")) {
        document.getElementById("paste-spec-section").classList.remove("hidden");
      }
      // Global hooks and the allow-list are instance configuration (#47).
      if (hasPermission(data, "manage_settings")) {
        document.getElementById("webhooks-section").classList.remove("hidden");
        loadWebhooks();
      }
    } else {
      window.location.href = "/account.html";
    }
  } catch (_) {
    window.location.href = "/account.html";
  }
}

// --- Instance settings (#harmless values, readable by everyone signed in) ---
// One fetch shows every harmless setting's real value, also to someone who may
// not change it: a greyed control must never show a value it did not receive.
let instanceSettingsRequest = null;
function instanceSettings(refresh = false) {
  if (refresh || !instanceSettingsRequest) {
    instanceSettingsRequest = apiCall("/admin/instance-settings")
      .then((res) => (res.ok ? res.json() : null))
      .catch(() => null);
  }
  return instanceSettingsRequest;
}

// A section whose content the caller may not read: say so instead of loading
// it and drawing a 403 as an empty or wrong value.
function lockedNote(elementId, permission) {
  const el = document.getElementById(elementId);
  if (el) {
    el.innerHTML = `<p class="text-sm text-slate-400 italic" data-locked>Visible to holders of the ${escapeHtml(permission)} permission.</p>`;
  }
}

// --- Tabs ---
// The page is split into General, Sign-in and Users. A tab whose permissions
// the person lacks entirely is greyed out and says which permission opens it.
// The open tab is in the address, so it can be linked, and remembered in this
// browser for the next visit.
const ADMIN_TABS = ["general", "sign-in", "users"];
const ADMIN_TAB_PERMISSIONS = {
  "sign-in": ["manage_auth_config", "manage_settings"],
  users: ["manage_users", "manage_roles"],
};
const ADMIN_TAB_KEY = "sanshain_admin_tab";

function adminTabAllowed(tab) {
  const needs = ADMIN_TAB_PERMISSIONS[tab];
  if (!needs) return true;
  return adminActor?.is_root || needs.some((p) => hasPermission(adminActor, p));
}

function paintAdminTabs(open) {
  document.querySelectorAll("[data-admin-tab-button]").forEach((button) => {
    const tab = button.dataset.adminTabButton;
    const allowed = adminTabAllowed(tab);
    const selected = tab === open;
    button.disabled = !allowed;
    button.title = allowed ? "" : `Needs ${ADMIN_TAB_PERMISSIONS[tab].join(" or ")}`;
    button.setAttribute("aria-selected", String(selected));
    button.className = `px-4 py-2 text-sm font-medium border-b-2 -mb-px ${
      !allowed
        ? "border-transparent text-slate-300 cursor-not-allowed"
        : selected
          ? "border-indigo-600 text-indigo-700"
          : "border-transparent text-slate-500 hover:text-slate-800"
    }`;
  });
  document.querySelectorAll("[data-admin-tab]").forEach((section) => {
    section.hidden = section.dataset.adminTab !== open;
  });
}

function openAdminTab(tab) {
  if (!ADMIN_TABS.includes(tab) || !adminTabAllowed(tab)) tab = "general";
  paintAdminTabs(tab);
  history.replaceState(null, "", `#${tab}`);
  try {
    localStorage.setItem(ADMIN_TAB_KEY, tab);
  } catch (_) {
    // Forgotten on the next visit; the tab still opens now.
  }
}

// A link to another tab on the open page (#users) switches to it.
window.addEventListener("hashchange", () => {
  const tab = location.hash.slice(1);
  if (ADMIN_TABS.includes(tab)) openAdminTab(tab);
});

function initialAdminTab() {
  const fromAddress = location.hash.slice(1);
  if (ADMIN_TABS.includes(fromAddress)) return fromAddress;
  try {
    return localStorage.getItem(ADMIN_TAB_KEY) || "general";
  } catch (_) {
    return "general";
  }
}

// --- Load All ---
async function loadAll() {
  openAdminTab(initialAdminTab());
  // Each section loads only what the caller may read; the rest says whose it is.
  const can = (permission) => adminActor?.is_root || hasPermission(adminActor, permission);
  if (can("manage_settings")) loadDatabaseInfo();
  else lockedNote("db-url", "manage_settings");
  loadCacheConfig();
  loadBranches();
  loadTrunkMaxAge();
  loadSnapshotMaxAge();
  loadTicketUrlTemplateSetting();
  loadDependencyMaxAge();
  loadServices();
  loadClients();
  loadAutoApproveUsers();
  if (can("manage_auth_config")) loadAuthConfig();
  else lockedNote("auth-config-locked", "manage_auth_config");
  // Ordered: the catalogue tells the user and group rows which roles
  // exist, the user list feeds the maintainer picker, and groups feed
  // it too — so each waits for what it needs.
  if (can("manage_roles")) await loadRoleCatalogue();
  if (can("manage_users")) await loadUsers();
  else lockedNote("users-list", "manage_users");
  if (can("manage_roles")) {
    await loadGroups();
    await loadMaintainers();
  } else {
    lockedNote("groups-list", "manage_roles");
    lockedNote("maintainers-list", "manage_roles");
  }
}

// --- Webhooks (#47) ---
async function loadWebhooks() {
  loadWebhookAllowedHosts();
  renderWebhookPanel(document.getElementById("webhooks-panel"), "/admin/webhooks");
}

async function loadWebhookAllowedHosts() {
  try {
    const res = await apiCall("/admin/settings/webhook-allowed-hosts");
    if (!res.ok) return;
    const data = await res.json();
    document.getElementById("webhook-allowed-hosts").value = (data.hosts || []).join("\n");
  } catch (_) {
    /* the textarea stays as it is */
  }
}

async function saveWebhookAllowedHosts() {
  const result = document.getElementById("webhook-allowed-hosts-result");
  const hosts = document
    .getElementById("webhook-allowed-hosts")
    .value.split("\n")
    .map((h) => h.trim())
    .filter(Boolean);
  try {
    const res = await apiCall("/admin/settings/webhook-allowed-hosts", {
      method: "POST",
      body: { hosts },
    });
    if (!res.ok) {
      result.textContent = "Not saved: " + (await errorMessage(res));
      result.className = "text-sm text-red-600";
      return;
    }
    const data = await res.json();
    document.getElementById("webhook-allowed-hosts").value = (data.hosts || []).join("\n");
    result.textContent = `Saved (${data.hosts.length} host pattern(s)).`;
    result.className = "text-sm text-green-600";
  } catch (err) {
    result.textContent = "Not saved: " + err.message;
    result.className = "text-sm text-red-600";
  }
}

// --- Database Info ---
async function loadDatabaseInfo() {
  try {
    const res = await apiCall("/admin/settings/database");
    if (res.ok) {
      const data = await res.json();
      document.getElementById("db-backend").textContent =
        data.backend === "postgres" ? "PostgreSQL" : "SQLite";
      document.getElementById("db-url").textContent = data.url;
      // The snapshot is SQLite-only and a whole-database export, so the
      // button shows only where the route would answer (#50).
      if (data.backend === "sqlite" && hasPermission(adminActor, "run_destructive_operations")) {
        document.getElementById("db-snapshot").classList.remove("hidden");
      }
    }
  } catch (_) {}
}

async function downloadDatabaseSnapshot() {
  const btn = document.getElementById("db-snapshot-btn");
  btn.disabled = true;
  try {
    const res = await apiCall("/admin/backup/sqlite");
    if (!res.ok) {
      alert("Snapshot failed: " + (await errorMessage(res)));
      return;
    }
    // The server names the file after the instance version and the time.
    const disposition = res.headers.get("Content-Disposition") || "";
    const named = disposition.match(/filename="([^"]+)"/);
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = named ? named[1] : "sanshain.db";
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch (e) {
    alert("Snapshot failed: " + friendlyError(e));
  } finally {
    btn.disabled = false;
  }
}

// --- Cache Config ---
async function loadCacheConfig() {
  try {
    const settings = await instanceSettings(true);
    if (settings) {
      const data = settings.cache;
      document.getElementById("cache-memory-mb").value = data.memory_limit_mb;
      if (data.enabled) {
        document.getElementById("cache-stats").classList.remove("hidden");
        document.getElementById("cache-disabled-msg").classList.add("hidden");
        document.getElementById("cache-hit-rate").textContent = `${data.hit_rate.toFixed(1)}%`;
        document.getElementById("cache-entries").textContent = data.entry_count;
        document.getElementById("cache-memory-used").textContent =
          `${data.memory_used_mb.toFixed(1)} MB`;
        document.getElementById("cache-hits-misses").textContent =
          `${data.hit_count} / ${data.miss_count}`;
      } else {
        document.getElementById("cache-stats").classList.add("hidden");
        document.getElementById("cache-disabled-msg").classList.remove("hidden");
      }
    }
  } catch (_) {}
}

async function saveCacheConfig() {
  const mb = parseInt(document.getElementById("cache-memory-mb").value, 10);
  if (isNaN(mb) || mb < 0) return;
  try {
    const res = await apiCall("/admin/settings/cache", { method: "POST", body: { memory_mb: mb } });
    if (res.ok) {
      document.getElementById("cache-action-result").textContent = "✓ Saved";
      setTimeout(() => (document.getElementById("cache-action-result").textContent = ""), 3000);
      loadCacheConfig();
    }
  } catch (_) {}
}

async function clearCache() {
  try {
    const res = await apiCall("/admin/cache/clear", { method: "POST", body: {} });
    if (res.ok) {
      document.getElementById("cache-action-result").textContent = "✓ Cache cleared";
      setTimeout(() => (document.getElementById("cache-action-result").textContent = ""), 3000);
      loadCacheConfig();
    }
  } catch (_) {}
}

// --- Snapshot Cleanup ---
// --- Trunk TTL (ADR-0004) ---
async function loadTrunkMaxAge() {
  try {
    const settings = await instanceSettings();
    if (settings) {
      document.getElementById("trunk-max-age-days").value = settings.trunk_max_age_days;
    }
  } catch (e) {
    console.warn("Could not load trunk max age:", e);
  }
}

async function saveTrunkMaxAge() {
  const days = parseInt(document.getElementById("trunk-max-age-days").value, 10);
  const res = await apiCall("/admin/settings/trunk-max-age", { method: "POST", body: { days } });
  document.getElementById("trunk-cleanup-result").textContent = res.ok ? "Saved." : "Save failed.";
}

async function triggerTrunkCleanup() {
  const res = await apiCall("/admin/cleanup/trunk", { method: "POST" });
  if (res.ok) {
    const data = await res.json();
    document.getElementById("trunk-cleanup-result").textContent =
      `Closed ${data.closed} stale trunk pins.`;
  } else {
    document.getElementById("trunk-cleanup-result").textContent = "Cleanup failed.";
  }
}

// --- Sanshain-branches (ADR-0005) ---
async function loadBranches() {
  const list = document.getElementById("branches-list");
  try {
    const res = await apiCall("/admin/branches");
    if (!res.ok) {
      list.innerHTML = '<div class="text-slate-400 italic">Could not load branches.</div>';
      return;
    }
    const branches = await res.json();
    if (branches.length === 0) {
      list.innerHTML = '<div class="text-slate-400 italic">No sanshain-branches yet.</div>';
      return;
    }
    list.innerHTML = "";
    for (const b of branches) {
      const row = document.createElement("div");
      row.className =
        "flex items-center justify-between gap-3 border border-slate-100 rounded-lg px-3 py-2";
      row.innerHTML = `
                        <div class="min-w-0">
                            <span class="font-semibold text-slate-800" data-branch-name></span>
                            <span class="text-slate-400 text-xs ml-2">from <span data-branch-source></span> as of <span data-branch-asof></span> by <span data-branch-creator></span></span>
                        </div>
                        <div class="flex items-center gap-2">
                            <button data-rename data-needs="manage_producers" class="px-2.5 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-slate-50 text-indigo-600">Rename</button>
                            <button data-delete data-needs="manage_producers" class="px-2.5 py-1 text-xs font-semibold bg-white border border-slate-200 rounded-lg hover:bg-red-50 text-red-600">Delete</button>
                        </div>`;
      row.querySelector("[data-branch-name]").textContent = b.name;
      row.querySelector("[data-branch-source]").textContent = b.source;
      row.querySelector("[data-branch-asof]").textContent = (b.as_of || "")
        .slice(0, 16)
        .replace("T", " ");
      row.querySelector("[data-branch-creator]").textContent = b.created_by;
      row.querySelector("[data-rename]").addEventListener("click", () => renameBranch(b.name));
      row
        .querySelector("[data-delete]")
        .addEventListener("click", () =>
          confirmDelete(
            `Delete sanshain-branch <strong>${escapeHtml(b.name)}</strong>? Its graph and timeline are removed and the name is freed.`,
            () => deleteBranch(b.name),
          ),
        );
      list.appendChild(row);
    }
  } catch (e) {
    list.innerHTML = `<div class="text-red-500">Failed to load branches: ${escapeHtml(e.message)}</div>`;
  }
}

async function createBranch() {
  const name = document.getElementById("new-branch-name").value.trim();
  if (!name) return;
  const asOfRaw = document.getElementById("new-branch-as-of").value;
  const body = { name };
  if (asOfRaw) body.as_of = new Date(asOfRaw).toISOString();
  const res = await apiCall("/admin/branches", { method: "POST", body });
  if (!res.ok) {
    alert("Could not create the branch: " + (await errorMessage(res)));
    return;
  }
  document.getElementById("new-branch-name").value = "";
  document.getElementById("new-branch-as-of").value = "";
  loadBranches();
}

async function renameBranch(name) {
  const newName = prompt(`Rename sanshain-branch '${name}' to:`, name);
  if (!newName || newName === name) return;
  const res = await apiCall(`/admin/branches/${encodeURIComponent(name)}`, {
    method: "PUT",
    body: { new_name: newName },
  });
  if (!res.ok) {
    alert("Could not rename: " + (await errorMessage(res)));
    return;
  }
  loadBranches();
}

async function deleteBranch(name) {
  const res = await apiCall(`/admin/branches/${encodeURIComponent(name)}`, { method: "DELETE" });
  if (!res.ok) {
    alert("Could not delete: " + (await errorMessage(res)));
    return;
  }
  loadBranches();
}

async function loadSnapshotMaxAge() {
  try {
    const settings = await instanceSettings();
    if (settings) {
      document.getElementById("snapshot-max-age-days").value = settings.snapshot_max_age_days;
    }
  } catch (_) {}
}

async function saveSnapshotMaxAge() {
  const days = parseInt(document.getElementById("snapshot-max-age-days").value, 10);
  if (isNaN(days) || days < 0) return;
  try {
    const res = await apiCall("/admin/settings/snapshot-max-age", {
      method: "POST",
      body: { days },
    });
    if (res.ok) {
      document.getElementById("snapshot-cleanup-result").textContent = "✓ Saved";
      setTimeout(() => (document.getElementById("snapshot-cleanup-result").textContent = ""), 3000);
    }
  } catch (_) {}
}

async function loadTicketUrlTemplateSetting() {
  try {
    const res = await apiCall("/admin/ticket-url-template");
    if (res.ok) {
      const data = await res.json();
      document.getElementById("ticket-url-template").value = data.template || "";
    }
  } catch (_) {}
}

async function saveTicketUrlTemplate() {
  const template = document.getElementById("ticket-url-template").value.trim();
  const result = document.getElementById("ticket-url-template-result");
  try {
    const res = await apiCall("/admin/settings/ticket-url-template", {
      method: "POST",
      body: { template: template || null },
    });
    if (res.ok) {
      result.textContent = template ? "✓ Saved" : "✓ Cleared";
    } else {
      result.textContent = "✗ " + (await errorMessage(res));
    }
    setTimeout(() => (result.textContent = ""), 5000);
  } catch (_) {}
}

async function triggerSnapshotCleanup() {
  try {
    const res = await apiCall("/admin/cleanup/snapshots", { method: "POST", body: {} });
    if (res.ok) {
      const data = await res.json();
      document.getElementById("snapshot-cleanup-result").textContent =
        `✓ Deleted ${data.deleted} snapshot${data.deleted !== 1 ? "s" : ""}`;
      setTimeout(() => (document.getElementById("snapshot-cleanup-result").textContent = ""), 5000);
      loadServices();
    }
  } catch (_) {}
}

// --- Dependency Cleanup ---
async function loadDependencyMaxAge() {
  try {
    const settings = await instanceSettings();
    if (settings) {
      document.getElementById("dependency-max-age-days").value = settings.dependency_max_age_days;
    }
  } catch (_) {}
}

async function saveDependencyMaxAge() {
  const days = parseInt(document.getElementById("dependency-max-age-days").value, 10);
  if (isNaN(days) || days < 1) return;
  try {
    const res = await apiCall("/admin/settings/dependency-max-age", {
      method: "POST",
      body: { days },
    });
    if (res.ok) {
      document.getElementById("dependency-cleanup-result").textContent = "✓ Saved";
      setTimeout(
        () => (document.getElementById("dependency-cleanup-result").textContent = ""),
        3000,
      );
    }
  } catch (_) {}
}

async function triggerDependencyCleanup() {
  try {
    const res = await apiCall("/admin/cleanup/dependencies", { method: "POST", body: {} });
    if (res.ok) {
      const data = await res.json();
      document.getElementById("dependency-cleanup-result").textContent =
        `✓ Deleted ${data.deleted} dependencies`;
      setTimeout(
        () => (document.getElementById("dependency-cleanup-result").textContent = ""),
        5000,
      );
      loadClients();
    }
  } catch (_) {}
}

// --- Local users: registration and auto-acknowledge ---
// Two switches that save on click. They apply in Local mode only, so the
// panel holding them shows only while Local is the selected mode.
const localUserSwitches = {
  registration: { url: "/admin/settings/registration", key: "registration_enabled", on: false },
  "auto-approve": { url: "/admin/settings/auto-approve", key: "auto_approve_users", on: false },
  gravatar: { url: "/admin/settings/gravatar", key: "gravatar_enabled", on: false },
};

async function loadAutoApproveUsers() {
  try {
    const settings = await instanceSettings();
    if (settings) {
      for (const [id, sw] of Object.entries(localUserSwitches)) {
        sw.on = !!settings[sw.key];
        paintSwitch(id);
      }
      devModeAllowed = !!settings.dev_mode_allowed;
      onAuthModeChange();
    }
  } catch (_) {}
}

function paintSwitch(id) {
  const on = localUserSwitches[id].on;
  const toggle = document.getElementById(`${id}-toggle`);
  const dot = document.getElementById(`${id}-dot`);
  toggle.classList.toggle("bg-indigo-600", on);
  toggle.classList.toggle("bg-slate-200", !on);
  dot.classList.toggle("translate-x-6", on);
  dot.classList.toggle("translate-x-1", !on);
  toggle.setAttribute("aria-checked", String(on));
}

async function flipSwitch(id) {
  const sw = localUserSwitches[id];
  try {
    const res = await apiCall(sw.url, { method: "POST", body: { enabled: !sw.on } });
    if (!res.ok) {
      alert("Failed to save: " + (await errorMessage(res)));
      return;
    }
    sw.on = !sw.on;
    paintSwitch(id);
    instanceSettings(true);
    loadUsers();
  } catch (e) {
    alert("Failed to save: " + e.message);
  }
}

function toggleRegistration() {
  return flipSwitch("registration");
}

function toggleAutoApproveUsers() {
  return flipSwitch("auto-approve");
}

function toggleGravatar() {
  return flipSwitch("gravatar");
}

// --- Roles, groups and maintainers ---
//
// Roles are fixed in the service, so this screen assigns them; it cannot
// compose new ones. What each role confers is shown so an administrator
// can see what they are handing over.
let roleCatalogue = [];
let knownUsers = [];
let knownGroups = [];

async function loadRoleCatalogue() {
  try {
    const res = await apiCall("/admin/roles");
    if (!res.ok) return;
    const data = await res.json();
    roleCatalogue = data.roles || [];
    const note = document.getElementById("root-note");
    if (note) {
      note.textContent =
        "Root is held by configuration (SANSHAIN_ROOT_USERS) and cannot be granted or revoked here.";
    }
  } catch (_) {}
}

function grantableRoles() {
  return roleCatalogue.filter((r) => r.globally_grantable);
}

function roleTitle(roleName) {
  const entry = roleCatalogue.find((r) => r.role === roleName);
  return entry ? `Confers: ${entry.permissions.join(", ")}` : "";
}

function roleControls(user) {
  const held = user.roles || [];
  const options = grantableRoles()
    .filter((r) => !held.includes(r.role))
    .map(
      (r) =>
        `<option value="${escapeHtml(r.role)}" title="${escapeHtml(roleTitle(r.role))}">${escapeHtml(r.role)}</option>`,
    )
    .join("");
  const grant = options
    ? `<select id="grant-role-${user.id}" class="text-xs rounded-sm border-slate-200">
                       <option value="">grant role…</option>${options}
                   </select>
                   <button data-click="grantRole" data-needs="manage_roles" data-click-args="${attrJson([user.id])}" class="text-indigo-600 hover:text-indigo-800 text-sm">Grant</button>`
    : "";
  const revoke = held
    .map(
      (
        r,
      ) => `<button data-click="revokeRole" data-needs="manage_roles" data-click-args="${attrJson([user.id, r])}"
                        title="Revoke ${escapeAttr(r)}"
                        class="text-xs text-slate-400 hover:text-red-600">✕ ${escapeHtml(r)}</button>`,
    )
    .join(" ");
  return grant + revoke;
}

async function grantRole(userId) {
  const select = document.getElementById(`grant-role-${userId}`);
  const role = select ? select.value : "";
  if (!role) return;
  const res = await apiCall(`/admin/users/${userId}/roles`, {
    method: "POST",
    body: { role },
  });
  if (!res.ok) {
    alert("Failed to grant: " + (await errorMessage(res)));
    return;
  }
  loadUsers();
}

async function revokeRole(userId, role) {
  // Warn rather than block: root is configured outside the database and
  // is the recovery path, so this is recoverable — but it is easy to do
  // by accident and hard to notice.
  const me = await currentActor();
  if (me && me.id === userId && (me.roles || []).length === 1) {
    if (
      !window.confirm(`Revoke your own last role (${role})? You will lose access to this page.`)
    ) {
      return;
    }
  }
  const res = await apiCall(`/admin/users/${userId}/roles/${encodeURIComponent(role)}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    alert("Failed to revoke: " + (await errorMessage(res)));
    return;
  }
  loadUsers();
}

async function currentActor() {
  try {
    const res = await apiCall("/auth/me");
    return res.ok ? await res.json() : null;
  } catch (_) {
    return null;
  }
}

async function loadGroups() {
  const list = document.getElementById("groups-list");
  if (!list) return;
  try {
    const res = await apiCall("/admin/groups");
    if (!res.ok) {
      list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
      return;
    }
    const data = await res.json();
    knownGroups = data.groups || [];
    if (knownGroups.length === 0) {
      list.innerHTML = '<p class="text-sm text-slate-500 italic">No groups yet.</p>';
      mountDirectoryGroupPicker();
      refreshMaintainerPickers();
      return;
    }
    list.innerHTML = knownGroups
      .map((g) => {
        const directory = g.source === "ldap";
        const badge = directory
          ? '<span class="text-xs bg-sky-100 text-sky-700 px-2 py-0.5 rounded-full">directory</span>'
          : '<span class="text-xs bg-slate-200 text-slate-700 px-2 py-0.5 rounded-full">sanshain</span>';
        const roles = (g.roles || [])
          .map(
            (
              r,
            ) => `<button data-click="setGroupRoles" data-needs="manage_roles" data-click-args="${attrJson([g.id, r, false])}"
                                title="Remove ${escapeAttr(r)}"
                                class="text-xs text-slate-400 hover:text-red-600">✕ ${escapeHtml(r)}</button>`,
          )
          .join(" ");
        const addable = grantableRoles()
          .filter((r) => !(g.roles || []).includes(r.role))
          .map(
            (r) =>
              `<option value="${escapeHtml(r.role)}" title="${escapeHtml(roleTitle(r.role))}">${escapeHtml(r.role)}</option>`,
          )
          .join("");
        const memberIds = g.member_ids || [];
        const members = directory
          ? '<span class="text-xs text-slate-400 italic">membership from the directory</span>'
          : `<span class="text-xs text-slate-500">${memberIds.length} member(s)</span>`;
        const memberChips = directory ? "" : groupMemberChips(g);
        return `
                    <div class="bg-slate-50 px-4 py-2 rounded-lg fade-in">
                        <div class="flex justify-between items-center">
                            <div class="flex items-center space-x-3">
                                <span class="font-medium text-slate-800">${escapeHtml(g.name)}</span>
                                ${badge}
                                ${members}
                            </div>
                            <div class="flex items-center space-x-2">
                                ${
                                  addable
                                    ? `<select id="group-role-${g.id}" class="text-xs rounded-sm border-slate-200">
                                    <option value="">add role…</option>${addable}
                                </select>
                                <button data-click="addGroupRole" data-needs="manage_roles" data-click-args="${attrJson([g.id])}" class="text-indigo-600 hover:text-indigo-800 text-sm">Add</button>`
                                    : ""
                                }
                                ${directory ? "" : `<button data-click="confirmDeleteGroup" data-needs="manage_roles" data-click-args="${attrJson([g.name, g.id])}" class="text-red-500 hover:text-red-700 text-sm">Delete</button>`}
                            </div>
                        </div>
                        <div class="mt-1 space-x-2">${roles || '<span class="text-xs text-slate-400 italic">no roles attached</span>'}</div>
                        ${
                          directory
                            ? ""
                            : `<div class="mt-2 flex flex-wrap items-center gap-2">
                            <span class="flex flex-wrap gap-2" data-member-chips="${g.id}">${memberChips}</span>
                            <div class="w-56" data-member-picker="${g.id}"></div>
                        </div>`
                        }
                    </div>`;
      })
      .join("");
    list.querySelectorAll("[data-member-picker]").forEach(mountGroupMemberPicker);
    mountDirectoryGroupPicker();
    refreshMaintainerPickers();
  } catch (_) {
    list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
  }
}

function groupMemberChips(g) {
  const chips = (g.member_ids || []).map((id) => {
    const u = knownUsers.find((x) => x.id === id);
    const label = u ? personLabel(u) : `user #${id}`;
    return `<button data-click="removeGroupMember" data-needs="manage_roles" data-click-args="${attrJson([g.id, id])}"
                                title="Remove ${escapeAttr(label)} from ${escapeAttr(g.name)}"
                                class="text-xs text-slate-500 hover:text-red-600 bg-white border border-slate-200 rounded-full px-2 py-0.5">✕ ${escapeHtml(label)}</button>`;
  });
  return chips.join(" ") || '<span class="text-xs text-slate-400 italic">no members</span>';
}

// Members are ticked into a Sanshain group and unticked out of it, each tick
// saved at once.
function mountGroupMemberPicker(root) {
  const groupId = Number(root.dataset.memberPicker);
  const group = () => knownGroups.find((g) => g.id === groupId);
  createPicker(root, {
    id: `group-member-${groupId}`,
    placeholder: "Add or remove members…",
    multiple: true,
    options: () => knownUsers.map((u) => ({ value: u.id, label: personLabel(u) })),
    isSelected: (id) => (group()?.member_ids || []).includes(id),
    onToggle: async (id, checked) => {
      const res = checked
        ? await apiCall(`/admin/groups/${groupId}/members`, {
            method: "POST",
            body: { user_id: id },
          })
        : await apiCall(`/admin/groups/${groupId}/members/${id}`, { method: "DELETE" });
      if (!res.ok) {
        alert(`Failed to ${checked ? "add" : "remove"} member: ${await errorMessage(res)}`);
        return;
      }
      const g = group();
      g.member_ids = checked
        ? [...(g.member_ids || []), id]
        : (g.member_ids || []).filter((x) => x !== id);
      document.querySelector(`[data-member-chips="${groupId}"]`).innerHTML = groupMemberChips(g);
    },
  });
}

async function createGroup() {
  const input = document.getElementById("new-group-name");
  const name = input ? input.value.trim() : "";
  if (!name) return;
  const res = await apiCall("/admin/groups", { method: "POST", body: { name } });
  if (!res.ok) {
    alert("Failed to create: " + (await errorMessage(res)));
    return;
  }
  input.value = "";
  loadGroups();
}

async function removeGroupMember(groupId, userId) {
  const res = await apiCall(`/admin/groups/${groupId}/members/${userId}`, { method: "DELETE" });
  if (!res.ok) {
    alert("Failed to remove member: " + (await errorMessage(res)));
    return;
  }
  loadGroups();
}

async function addGroupRole(groupId) {
  const select = document.getElementById(`group-role-${groupId}`);
  const role = select ? select.value : "";
  if (!role) return;
  await setGroupRoles(groupId, role, true);
}

// The endpoint replaces the whole set, so the current roles are read and
// the one change applied to them.
async function setGroupRoles(groupId, role, add) {
  const group = knownGroups.find((g) => g.id === groupId);
  const current = group ? group.roles || [] : [];
  const roles = add ? current.concat([role]) : current.filter((r) => r !== role);
  const res = await apiCall(`/admin/groups/${groupId}`, { method: "PUT", body: { roles } });
  if (!res.ok) {
    alert("Failed to update roles: " + (await errorMessage(res)));
    return;
  }
  loadGroups();
}

async function deleteGroup(groupId) {
  const res = await apiCall(`/admin/groups/${groupId}`, { method: "DELETE" });
  if (!res.ok) {
    alert("Failed to delete: " + (await errorMessage(res)));
    return;
  }
  loadGroups();
}

// Directory groups are found by searching LDAP while typing, so a team such
// as "carla" can be added without knowing its DN. Answers for a configured
// LDAP only; anything else leaves the search empty.
let directoryHits = [];
let directorySearchTimer = null;
function searchDirectoryGroups(text, then) {
  clearTimeout(directorySearchTimer);
  if (text.length < 2) {
    directoryHits = [];
    then();
    return;
  }
  directorySearchTimer = setTimeout(async () => {
    try {
      const res = await apiCall(`/admin/directory-groups?q=${encodeURIComponent(text)}`);
      directoryHits = res.ok ? (await res.json()).groups || [] : [];
    } catch (_) {
      directoryHits = [];
    }
    then();
  }, 250);
}

// The directory hits Sanshain does not know yet, as picker entries.
function newDirectoryOptions() {
  return directoryHits
    .filter((hit) => hit.group_id === null)
    .map((hit) => ({
      value: `dn:${hit.dn}`,
      label: `${hit.name} (directory)`,
      group: "Directory",
    }));
}

async function addDirectoryGroup(dn) {
  const res = await apiCall("/admin/groups/directory", { method: "POST", body: { dn } });
  if (!res.ok) {
    alert("Could not add the directory group: " + (await errorMessage(res)));
    return null;
  }
  const group = await res.json();
  directoryHits = directoryHits.map((h) => (h.dn === dn ? { ...h, group_id: group.id } : h));
  if (!knownGroups.some((g) => g.id === group.id)) {
    knownGroups.push({ ...group, roles: [], member_ids: [] });
  }
  return group;
}

let directoryGroupPicker = null;
function mountDirectoryGroupPicker() {
  const root = document.getElementById("directory-group-picker");
  if (!root || directoryGroupPicker) return;
  directoryGroupPicker = createPicker(root, {
    id: "directory-group-search",
    placeholder: "Type a team name to search the directory…",
    multiple: false,
    options: newDirectoryOptions,
    isSelected: () => false,
    onSearch: (text) => searchDirectoryGroups(text, () => directoryGroupPicker.refresh()),
    onPick: async (value) => {
      if (await addDirectoryGroup(value.slice(3))) loadGroups();
    },
    selectedLabel: () => "",
  });
}

// Maintainers: pick a Producer, then tick the users and groups responsible
// for it. Each tick or untick is saved at once. A directory group found by
// typing its name is added to Sanshain on the tick.
let maintainerData = [];
let maintainerProducer = "";
let maintainerProducerPicker = null;
let maintainerSubjectPicker = null;

function maintainersOf(producer) {
  return maintainerData.find((p) => p.producer === producer) || { user_ids: [], group_ids: [] };
}

function refreshMaintainerPickers() {
  const producerRoot = document.getElementById("maintainer-producer-picker");
  const subjectRoot = document.getElementById("maintainer-subject-picker");
  if (!producerRoot || !subjectRoot) return;
  if (!maintainerProducerPicker) {
    maintainerProducerPicker = createPicker(producerRoot, {
      id: "maintainer-producer",
      placeholder: "Pick a Producer…",
      multiple: false,
      options: () => maintainerData.map((p) => ({ value: p.producer, label: p.producer })),
      isSelected: (value) => value === maintainerProducer,
      onPick: (value) => {
        maintainerProducer = value;
        maintainerSubjectPicker.refresh();
      },
      selectedLabel: () => maintainerProducer,
    });
    maintainerSubjectPicker = createPicker(subjectRoot, {
      id: "maintainer-subject",
      placeholder: "Tick users and groups…",
      multiple: true,
      options: () =>
        maintainerProducer
          ? knownUsers
              .map((u) => ({ value: `user:${u.id}`, label: personLabel(u), group: "Users" }))
              .concat(
                knownGroups.map((g) => ({
                  value: `group:${g.id}`,
                  label: g.source === "ldap" ? `${g.name} (directory)` : g.name,
                  group: "Groups",
                })),
              )
              .concat(newDirectoryOptions())
          : [],
      onSearch: (text) => searchDirectoryGroups(text, () => maintainerSubjectPicker.refresh()),
      isSelected: (value) => {
        const [kind, id] = value.split(":");
        const m = maintainersOf(maintainerProducer);
        return (kind === "user" ? m.user_ids : m.group_ids || []).includes(Number(id));
      },
      onToggle: async (value, checked) => {
        if (value.startsWith("dn:")) {
          const group = await addDirectoryGroup(value.slice(3));
          if (group) await setMaintainer(maintainerProducer, `group:${group.id}`, true);
          return;
        }
        await setMaintainer(maintainerProducer, value, checked);
      },
    });
  }
  maintainerProducerPicker.refresh();
  maintainerSubjectPicker.refresh();
}

// One request fills the whole section: /admin/maintainers answers for
// every Producer at once. Asking per Producer meant 26 sequential
// round-trips for 25 Producers.
async function loadMaintainers() {
  const list = document.getElementById("maintainers-list");
  if (!list) return;
  try {
    const res = await apiCall("/admin/maintainers");
    if (!res.ok) {
      list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
      return;
    }
    const data = await res.json();
    maintainerData = data.producers || [];
    refreshMaintainerPickers();
    renderMaintainers();
  } catch (_) {
    list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
  }
}

function renderMaintainers() {
  const list = document.getElementById("maintainers-list");
  const rows = maintainerData
    .filter((p) => (p.user_ids || []).length || (p.group_ids || []).length)
    .map((p) => {
      const users = (p.user_ids || []).map((id) => {
        const u = knownUsers.find((x) => x.id === id);
        return `<button data-click="unassignMaintainer" data-needs="manage_roles" data-click-args="${attrJson([p.producer, "users", id])}"
                                class="text-xs text-slate-400 hover:text-red-600">✕ ${escapeHtml(u ? personLabel(u) : "user " + id)}</button>`;
      });
      const groups = (p.group_ids || []).map((id) => {
        const g = knownGroups.find((x) => x.id === id);
        return `<button data-click="unassignMaintainer" data-needs="manage_roles" data-click-args="${attrJson([p.producer, "groups", id])}"
                                class="text-xs text-slate-400 hover:text-red-600">✕ ${escapeHtml(g ? g.name : "group " + id)}</button>`;
      });
      return `
                        <div class="bg-slate-50 px-4 py-2 rounded-lg fade-in" data-maintained="${escapeAttr(p.producer)}">
                            <span class="font-medium text-slate-800">${escapeHtml(p.producer)}</span>
                            <span class="ml-3 space-x-2">${users.concat(groups).join(" ")}</span>
                        </div>`;
    });
  list.innerHTML = rows.length
    ? rows.join("")
    : '<p class="text-sm text-slate-500 italic">No Producer has a maintainer yet.</p>';
}

async function setMaintainer(producer, subject, assign) {
  const [kind, id] = subject.split(":");
  const res = assign
    ? await apiCall(`/admin/producers/${encodeURIComponent(producer)}/maintainers`, {
        method: "POST",
        body: kind === "user" ? { user_id: Number(id) } : { group_id: Number(id) },
      })
    : await apiCall(`/admin/producers/${encodeURIComponent(producer)}/maintainers/${kind}s/${id}`, {
        method: "DELETE",
      });
  if (!res.ok) {
    alert(`Failed to ${assign ? "assign" : "unassign"}: ${await errorMessage(res)}`);
    return;
  }
  const m = maintainerData.find((p) => p.producer === producer);
  if (m) {
    const key = kind === "user" ? "user_ids" : "group_ids";
    m[key] = assign
      ? [...(m[key] || []), Number(id)]
      : (m[key] || []).filter((x) => x !== Number(id));
  }
  renderMaintainers();
}

async function unassignMaintainer(producer, kind, id) {
  const res = await apiCall(
    `/admin/producers/${encodeURIComponent(producer)}/maintainers/${kind}/${id}`,
    { method: "DELETE" },
  );
  if (!res.ok) {
    alert("Failed to unassign: " + (await errorMessage(res)));
    return;
  }
  loadMaintainers();
}

// --- Users ---
// What kind of account a row is: a service account, or how the person last
// signed in. "unknown" until their first sign-in since that was recorded.
const ACCOUNT_KIND_TAGS = {
  service: ["service account", "bg-slate-200 text-slate-700"],
  local: ["local", "bg-emerald-100 text-emerald-700"],
  ldap: ["LDAP", "bg-sky-100 text-sky-700"],
  oidc: ["OIDC", "bg-violet-100 text-violet-700"],
  unknown: ["unknown", "bg-slate-100 text-slate-400"],
};
function accountKindTag(user) {
  const kind = ACCOUNT_KIND_TAGS[user.kind] ? user.kind : "unknown";
  const [label, colours] = ACCOUNT_KIND_TAGS[kind];
  const marker = kind === "service" ? " data-service-account" : "";
  return `<span class="text-xs ${colours} px-2 py-0.5 rounded-full" data-account-kind="${kind}"${marker}>${label}</span>`;
}
async function loadUsers() {
  const list = document.getElementById("users-list");
  try {
    const res = await apiCall("/admin/users");
    if (!res.ok) {
      list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
      return;
    }
    knownUsers = await res.json();
    renderUsers();
  } catch (_) {
    list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
  }
}

// --- The user list: search, chips, and pending first ---
// Hundreds of accounts are no list to scroll. With nothing searched and no
// chip set, only the accounts waiting for approval show, plus how many more
// there are. Chips of different kinds narrow together; chips of one kind widen.
const userFilter = { text: "", kind: new Set(), status: new Set(), role: new Set() };
const USER_KIND_CHIPS = ["service", "local", "ldap", "oidc", "unknown"];

function userMatches(u) {
  const text = userFilter.text.toLowerCase();
  if (
    text &&
    !u.username.toLowerCase().includes(text) &&
    !(u.display_name || "").toLowerCase().includes(text)
  ) {
    return false;
  }
  if (userFilter.kind.size && !userFilter.kind.has(u.kind)) return false;
  const status = u.approved ? "approved" : "pending";
  if (userFilter.status.size && !userFilter.status.has(status)) return false;
  if (userFilter.role.size && !(u.roles || []).some((r) => userFilter.role.has(r))) return false;
  return true;
}

function userFilterActive() {
  return Boolean(
    userFilter.text || userFilter.kind.size || userFilter.status.size || userFilter.role.size,
  );
}

function renderUserChips() {
  const chip = (group, value, label) => {
    const on = userFilter[group].has(value);
    return `<button type="button" data-click="toggleUserChip" data-click-args="${attrJson([group, value])}" data-user-chip="${group}:${escapeAttr(value)}" aria-pressed="${on}"
      class="text-xs px-2 py-0.5 rounded-full border ${on ? "bg-indigo-600 border-indigo-600 text-white" : "bg-white border-slate-300 text-slate-600 hover:bg-slate-50"}">${escapeHtml(label)}</button>`;
  };
  const roles = [...new Set(knownUsers.flatMap((u) => u.roles || []))].sort();
  const group = (title, chips) =>
    `<span class="text-[10px] font-bold uppercase tracking-wider text-slate-400 ml-1">${title}</span>${chips.join("")}`;
  document.getElementById("users-chips").innerHTML = [
    group(
      "Kind",
      USER_KIND_CHIPS.map((k) => chip("kind", k, ACCOUNT_KIND_TAGS[k][0])),
    ),
    group("Status", [chip("status", "pending", "pending"), chip("status", "approved", "approved")]),
    roles.length
      ? group(
          "Role",
          roles.map((r) => chip("role", r, r)),
        )
      : "",
  ].join("");
}

function toggleUserChip(group, value) {
  const set = userFilter[group];
  if (set.has(value)) set.delete(value);
  else set.add(value);
  renderUsers();
}

function renderUsers() {
  const list = document.getElementById("users-list");
  const more = document.getElementById("users-more");
  renderUserChips();
  if (knownUsers.length === 0) {
    list.innerHTML = '<p class="text-sm text-slate-500 italic">No users registered.</p>';
    more.classList.add("hidden");
    return;
  }
  const shown = userFilterActive()
    ? knownUsers.filter(userMatches)
    : knownUsers.filter((u) => !u.approved);
  list.innerHTML = shown.length
    ? shown.map(userRow).join("")
    : `<p class="text-sm text-slate-500 italic">${userFilterActive() ? "No user matches." : "Nobody is waiting for approval."}</p>`;
  const hidden = knownUsers.length - shown.length;
  more.textContent = userFilterActive()
    ? `${shown.length} of ${knownUsers.length} users.`
    : `${hidden} more user${hidden === 1 ? "" : "s"}: search or pick a filter.`;
  more.classList.toggle("hidden", hidden === 0 && !userFilterActive());
}

function userRow(u) {
  return `
                    <div class="flex justify-between items-center bg-slate-50 px-4 py-2 rounded-lg fade-in">
                        <div class="flex items-center space-x-3">
                            ${avatarHtml(u)}
                            ${
                              u.display_name
                                ? `<span class="font-medium text-slate-800" data-display-name>${escapeHtml(u.display_name)}</span>
                                   <span class="text-xs text-slate-400" data-username>${escapeHtml(u.username)}</span>`
                                : `<span class="font-medium text-slate-800" data-username>${escapeHtml(u.username)}</span>`
                            }
                            ${accountKindTag(u)}
                            ${(u.roles || []).map((r) => `<span class="text-xs bg-indigo-100 text-indigo-700 px-2 py-0.5 rounded-full">${escapeHtml(r)}</span>`).join(" ")}
                            ${
                              u.approved
                                ? '<span class="text-xs bg-green-100 text-green-700 px-2 py-0.5 rounded-full">approved</span>'
                                : '<span class="text-xs bg-amber-100 text-amber-700 px-2 py-0.5 rounded-full">pending</span>'
                            }
                        </div>
                        <div class="flex items-center space-x-2">
                            ${u.service_account ? `<button data-click="renameServiceAccount" data-needs="manage_users" data-click-args="${attrJson([u.id, u.display_name || ""])}" class="text-slate-500 hover:text-indigo-700 text-sm">Name</button>` : ""}
                            ${roleControls(u)}
                            ${!u.approved ? `<button data-click="approveUser" data-needs="manage_users" data-click-args="${attrJson([u.id])}" class="text-green-600 hover:text-green-800 text-sm">Approve</button>` : ""}
                            ${!(u.roles || []).includes("admin") ? `<button data-click="confirmDeleteUser" data-needs="manage_users" data-click-args="${attrJson([u.username, u.id])}" class="text-red-500 hover:text-red-700 text-sm">Delete</button>` : ""}
                        </div>
                    </div>
                `;
}

document.getElementById("users-search").addEventListener("input", (e) => {
  userFilter.text = e.target.value.trim();
  renderUsers();
});

document.getElementById("service-account-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const input = document.getElementById("service-account-name");
  try {
    const res = await apiCall("/admin/users", {
      method: "POST",
      body: { username: input.value.trim() },
    });
    if (!res.ok) {
      alert("Could not create the service account: " + (await errorMessage(res)));
      return;
    }
    // Show the new account: it is approved, so the default view would hide it.
    const search = document.getElementById("users-search");
    search.value = input.value.trim();
    userFilter.text = search.value;
    input.value = "";
    loadUsers();
  } catch (err) {
    alert("Could not create the service account: " + err.message);
  }
});

// A service account's display name, set by whoever manages users.
async function renameServiceAccount(id, current) {
  const name = prompt("Display name (empty shows the account name):", current);
  if (name === null) return;
  try {
    const res = await apiCall(`/admin/users/${id}/display-name`, {
      method: "PUT",
      body: { display_name: name },
    });
    if (!res.ok) {
      alert("Could not set the display name: " + (await errorMessage(res)));
      return;
    }
    loadUsers();
  } catch (err) {
    alert("Could not set the display name: " + err.message);
  }
}
async function approveUser(id) {
  try {
    await apiCall(`/admin/users/${id}/approve`, { method: "POST" });
    loadUsers();
  } catch (_) {}
}

async function deleteUser(id) {
  try {
    await apiCall(`/admin/users/${id}`, { method: "DELETE" });
    loadUsers();
  } catch (_) {}
}

// --- Paste-a-spec Producer Creation ---
// The same /provide* endpoints every client uses: no placeholder
// content, no side door. The signed-in admin is the authenticated
// Actor, so attribution lands on their name.
document.getElementById("paste-spec-ga").addEventListener("change", (e) => {
  document.getElementById("paste-spec-ga-warning").classList.toggle("hidden", !e.target.checked);
});

async function pasteSpecSubmit(dryRun) {
  const name = document.getElementById("paste-spec-name").value.trim();
  const apiType = document.getElementById("paste-spec-type").value;
  const content = document.getElementById("paste-spec-content").value;
  const ga = document.getElementById("paste-spec-ga").checked;
  const resultEl = document.getElementById("paste-spec-result");
  const show = (cls, text) => {
    resultEl.classList.remove("hidden");
    resultEl.className = "mt-3 text-sm " + cls;
    resultEl.textContent = text;
  };
  if (!name) {
    show("text-red-600", "Producer name is required.");
    return;
  }
  if (!content.trim()) {
    show("text-red-600", "Paste a specification first.");
    return;
  }

  const route =
    apiType === "asyncapi"
      ? "/provide/asyncapi"
      : apiType === "proto"
        ? "/provide/grpc"
        : "/provide";
  const field =
    apiType === "asyncapi"
      ? "asyncapi_yaml"
      : apiType === "proto"
        ? "proto_content"
        : "openapi_yaml";
  const body = {
    producername: name,
    stability: ga ? "ga" : "snapshot",
    dry_run: dryRun,
  };
  body[field] = content;

  // A Producer this provide creates is someone else's API: mark it external.
  const isNew = !allServices.some((s) => (s.name || s) === name);
  try {
    const res = await apiCall(route, { method: "POST", body });
    if (res.ok) {
      const data = await res.json();
      if (!dryRun && isNew) await setProducerExternal(name, true);
      const c = data.changes || {};
      show(
        "text-green-600",
        `${dryRun ? "✓ Valid — would publish" : "✓ Published"} ${name} ` +
          `${data.version} as ${data.stability} ` +
          `(+${c.inserts ?? 0} ~${c.updates ?? 0} -${c.deletes ?? 0})`,
      );
      if (!dryRun) {
        document.getElementById("paste-spec-content").value = "";
        loadServices();
      }
    } else {
      const msg = await errorMessage(res);
      show("text-red-600", (dryRun ? "Rejected (dry run): " : "Rejected: ") + msg);
    }
  } catch (e) {
    show("text-red-600", "Error: " + e.message);
  }
}

// --- Services ---
let allServices = [];

async function loadServices() {
  const list = document.getElementById("services-list");
  try {
    const res = await apiCall("/admin/producers");
    if (!res.ok) {
      list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
      return;
    }
    allServices = await res.json();
    renderServices(allServices);
  } catch (_) {
    list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
  }
}

function filterServices() {
  const q = document.getElementById("services-filter").value.toLowerCase().trim();
  const filtered = q
    ? allServices.filter((s) => (s.name || s).toLowerCase().includes(q))
    : allServices;
  renderServices(filtered);
}

function renderServices(services) {
  const list = document.getElementById("services-list");
  const countEl = document.getElementById("services-count");
  countEl.textContent = `${services.length} of ${allServices.length} services`;
  if (services.length === 0) {
    list.innerHTML = '<p class="text-sm text-slate-500 italic">No services found.</p>';
    return;
  }
  list.innerHTML = "";
  for (const svcObj of services) {
    const svc = svcObj.name || svcObj;
    const div = document.createElement("div");
    div.className = "bg-slate-50 rounded-lg p-4 fade-in";
    div.innerHTML = `
                    <div class="flex justify-between items-center">
                        <div class="flex items-center space-x-3">
                            <button data-click="toggleVersions" data-click-args="${attrJson(["$this", svc])}" class="text-slate-500 hover:text-slate-600 text-sm">▶</button>
                            <span class="font-medium text-slate-800">${escapeHtml(svc)}</span>
                            ${svcObj.domain ? `<span class="text-[10px] bg-blue-50 text-blue-600 px-2 py-0.5 rounded-full border border-blue-100 font-bold uppercase">${escapeHtml(svcObj.domain)}</span>` : ""}
                            ${externalTag(svcObj)}
                        </div>
                        <div class="flex items-center space-x-3">
                            <button data-click="setProducerExternal" data-needs="manage_producers" data-click-args="${attrJson([svc, !svcObj.external])}"
                                class="text-slate-500 hover:text-slate-800 text-sm">${svcObj.external ? "Mark as ours" : "Mark external"}</button>
                            <button data-click="openMetadataModal" data-needs="manage_producers" data-click-args="${attrJson([svc, svcObj.icon || "", svcObj.domain || ""])}"
                                class="text-indigo-600 hover:text-indigo-800 text-sm">Metadata</button>
                            <button data-click="confirmDeleteService" data-needs="manage_producers" data-needs-producer="${escapeHtml(svc)}" data-click-args="${attrJson([svc])}"
                                class="text-red-500 hover:text-red-700 text-sm">Delete</button>
                        </div>
                    </div>
                    <div class="versions-container hidden mt-3 ml-8 space-y-1"></div>
                `;
    list.appendChild(div);
  }
}

// External: an outside API the company does not maintain.
async function setProducerExternal(name, external) {
  try {
    const res = await apiCall(`/admin/producers/${encodeURIComponent(name)}/external`, {
      method: "PUT",
      body: { external },
    });
    if (!res.ok) {
      alert("Failed to save: " + (await errorMessage(res)));
      return;
    }
    loadServices();
  } catch (e) {
    alert("Failed to save: " + e.message);
  }
}

async function toggleVersions(btn, serviceName) {
  const container = btn.closest(".bg-slate-50").querySelector(".versions-container");
  if (!container.classList.contains("hidden")) {
    container.classList.add("hidden");
    btn.textContent = "▶";
    return;
  }
  btn.textContent = "▼";
  container.classList.remove("hidden");
  await renderVersions(container, serviceName);
}

// Fetch-and-render for one Producer's versions panel, callable
// directly (post-promote refresh) without driving the toggle button.
async function renderVersions(container, serviceName) {
  container.innerHTML = '<p class="text-xs text-slate-500">Loading...</p>';
  try {
    const res = await apiCall(`/admin/producers/${encodeURIComponent(serviceName)}/versions`);
    if (!res.ok) {
      container.innerHTML = '<p class="text-xs text-red-500">Failed</p>';
      return;
    }
    const versions = await res.json();
    if (versions.length === 0) {
      container.innerHTML = '<p class="text-xs text-slate-500 italic">No versions</p>';
      return;
    }
    container.innerHTML = versions
      .map(
        (v) => `
                    <div class="flex justify-between items-center bg-white px-3 py-1.5 rounded-sm border border-slate-100 fade-in">
                        <span class="flex items-center gap-2">
                            <span class="text-[9px] font-bold uppercase tracking-wider text-slate-400">${escapeHtml(v.api_type)}</span>
                            <span class="text-sm font-mono text-slate-600">${escapeHtml(v.version)}</span>
                            ${
                              v.stability === "ga"
                                ? '<span class="text-[10px] text-green-700 bg-green-100 px-1.5 py-0.5 rounded-full font-bold uppercase">GA</span>'
                                : '<span class="text-[10px] text-amber-700 bg-amber-100 px-1.5 py-0.5 rounded-full font-bold uppercase">snapshot</span>'
                            }
                            <span class="text-xs text-slate-400">${v.endpoint_count} endpoint${v.endpoint_count !== 1 ? "s" : ""}</span>
                        </span>
                        <span class="flex items-center gap-3">
                            ${
                              v.stability !== "ga" && hasPermission(adminActor, "release_ga")
                                ? `<button data-click="promoteVersionRow" data-click-args="${attrJson(["$this", serviceName, v.api_type, v.version])}"
                                    class="text-green-600 hover:text-green-700 text-xs">Promote to GA</button>`
                                : ""
                            }
                            <button data-click="deleteVersion" data-needs="manage_producers" data-needs-producer="${escapeHtml(serviceName)}" data-click-args="${attrJson([serviceName, v.api_type, v.version])}"
                                class="text-red-400 hover:text-red-600 text-xs">Delete</button>
                        </span>
                    </div>
                `,
      )
      .join("");
  } catch (_) {
    container.innerHTML = '<p class="text-xs text-red-500">Failed</p>';
  }
}

async function deleteService(name) {
  try {
    await apiCall(`/admin/producers/${encodeURIComponent(name)}`, { method: "DELETE" });
    loadServices();
  } catch (_) {}
}

// The sole escape hatch from GA immutability: fetch the dependents
// first so the confirmation names the Consumers whose builds break.
async function deleteVersion(service, apiType, version) {
  let dependents = [];
  try {
    const res = await apiCall(
      `/admin/producers/${encodeURIComponent(service)}/versions/${encodeURIComponent(apiType)}/${encodeURIComponent(version)}/dependents`,
    );
    if (res.ok) dependents = await res.json();
  } catch (_) {}
  const warning =
    dependents.length > 0
      ? `<br><br><span class="text-red-600 font-semibold">Pinned Consumers (their requires fail with 404) and referencing sanshain-branches (left dangling):</span><br>${dependents.map((d) => escapeHtml(d)).join(", ")}`
      : "<br><br>No Consumers are pinned to this version.";
  confirmDelete(
    `Delete ${escapeHtml(apiType)} version <strong>${escapeHtml(version)}</strong> of <strong>${escapeHtml(service)}</strong>?${warning}`,
    async () => {
      const res = await apiCall(
        `/admin/producers/${encodeURIComponent(service)}/versions/${encodeURIComponent(apiType)}/${encodeURIComponent(version)}`,
        { method: "DELETE" },
      );
      if (!res.ok) {
        alert("Failed to delete: " + (await errorMessage(res)));
        return;
      }
      loadServices();
    },
  );
}

// One-click release (#28): shared flow from common.js; this wrapper
// only supplies the page's refresh (re-render the open versions panel
// so the badge flips).
function promoteVersionRow(btn, service, apiType, version) {
  const container = btn.closest(".bg-slate-50")?.querySelector(".versions-container");
  promoteVersion(service, apiType, version, async () => {
    if (container) {
      await renderVersions(container, service);
    }
  });
}

// --- Clients ---
let allClients = [];

async function loadClients() {
  const list = document.getElementById("clients-list");
  try {
    const res = await apiCall("/admin/consumers");
    if (!res.ok) {
      list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
      return;
    }
    allClients = await res.json();
    renderClients(allClients);
  } catch (_) {
    list.innerHTML = '<p class="text-sm text-red-500">Failed to load</p>';
  }
}

function filterClients() {
  const q = document.getElementById("clients-filter").value.toLowerCase().trim();
  const filtered = q ? allClients.filter((c) => c.toLowerCase().includes(q)) : allClients;
  renderClients(filtered);
}

function renderClients(clients) {
  const list = document.getElementById("clients-list");
  const countEl = document.getElementById("clients-count");
  countEl.textContent = `${clients.length} of ${allClients.length} clients`;
  if (clients.length === 0) {
    list.innerHTML = '<p class="text-sm text-slate-500 italic">No clients found.</p>';
    return;
  }
  list.innerHTML = clients
    .map(
      (c) => `
                <div class="flex justify-between items-center bg-slate-50 px-4 py-2 rounded-lg fade-in">
                    <span class="font-medium text-slate-800">${escapeHtml(c)}</span>
                    <button data-click="confirmDeleteClient" data-needs="manage_consumers" data-click-args="${attrJson([c])}"
                        class="text-red-500 hover:text-red-700 text-sm">Delete</button>
                </div>
            `,
    )
    .join("");
}

async function deleteClient(name) {
  try {
    await apiCall(`/admin/consumers/${encodeURIComponent(name)}`, { method: "DELETE" });
    loadClients();
  } catch (_) {}
}

// --- Auth Config ---
// Whether the server lets Dev mode be chosen (ALLOW_INSECURE_DEV_MODE).
let devModeAllowed = false;

const AUTH_MODE_HELP = {
  disabled:
    "Maintenance: the API answers 503 and only administrators sign in, through the source that was active before or a local password.",
  dev: "Provide, require and catalog work without tokens. Only administrators sign in.",
  local: "Local accounts with passwords. Tokens are required.",
  ldap: "Passwords are checked against the directory; a local password is checked first. Tokens are required.",
  oidc: "People sign in through the identity provider; local passwords keep working. Tokens are required.",
};

const OIDC_FIELDS = {
  issuer_url: "oidc-issuer-url",
  client_id: "oidc-client-id",
  client_secret: "oidc-client-secret",
  redirect_url: "oidc-redirect-url",
  username_claim: "oidc-username-claim",
  groups_claim: "oidc-groups-claim",
  admin_group: "oidc-admin-group",
};

async function loadAuthConfig() {
  try {
    const res = await apiCall("/admin/auth-config");
    if (res.ok) {
      const data = await res.json();
      const radios = document.querySelectorAll('input[name="auth-mode"]');
      radios.forEach((r) => (r.checked = r.value === data.auth_mode));
      onAuthModeChange();
      if (data.ldap_config) {
        document.getElementById("ldap-server-url").value = data.ldap_config.server_url || "";
        document.getElementById("ldap-bind-dn").value = data.ldap_config.bind_dn || "";
        document.getElementById("ldap-bind-password").value = data.ldap_config.bind_password || "";
        document.getElementById("ldap-base-dn").value = data.ldap_config.base_dn || "";
        document.getElementById("ldap-user-filter").value = data.ldap_config.user_filter || "";
        document.getElementById("ldap-username-attribute").value =
          data.ldap_config.username_attribute || "";
        document.getElementById("ldap-admin-group").value = data.ldap_config.admin_group || "";
      }
      if (data.oidc_config) {
        for (const [key, id] of Object.entries(OIDC_FIELDS)) {
          document.getElementById(id).value = data.oidc_config[key] || "";
        }
      }
    }
  } catch (_) {}
}

function onAuthModeChange() {
  const selected = document.querySelector('input[name="auth-mode"]:checked');
  const mode = selected ? selected.value : "";
  document.getElementById("ldap-config-form").classList.toggle("hidden", mode !== "ldap");
  document.getElementById("oidc-config-form").classList.toggle("hidden", mode !== "oidc");
  document.getElementById("local-users-panel").classList.toggle("hidden", mode !== "local");
  document.getElementById("auth-mode-help").textContent = AUTH_MODE_HELP[mode] || "";
  const devRadio = document.querySelector('input[name="auth-mode"][value="dev"]');
  // Dev stays selectable when it is already the mode, so the page never
  // shows no mode at all.
  devRadio.disabled = !devModeAllowed && !devRadio.checked;
  document.getElementById("dev-mode-hint").classList.toggle("hidden", devModeAllowed);
}

function getLdapConfigFromForm() {
  return {
    server_url: document.getElementById("ldap-server-url").value,
    bind_dn: document.getElementById("ldap-bind-dn").value,
    bind_password: document.getElementById("ldap-bind-password").value || null,
    base_dn: document.getElementById("ldap-base-dn").value,
    user_filter: document.getElementById("ldap-user-filter").value || "(uid={username})",
    username_attribute: document.getElementById("ldap-username-attribute").value.trim(),
    admin_group: document.getElementById("ldap-admin-group").value,
  };
}

function getOidcConfigFromForm() {
  const config = {};
  for (const [key, id] of Object.entries(OIDC_FIELDS)) {
    config[key] = document.getElementById(id).value.trim();
  }
  config.client_secret = config.client_secret || null;
  if (!config.username_claim) delete config.username_claim;
  if (!config.groups_claim) delete config.groups_claim;
  return config;
}

async function saveAuthConfig() {
  const selected = document.querySelector('input[name="auth-mode"]:checked');
  if (!selected) return;
  const body = { auth_mode: selected.value };
  if (selected.value === "ldap") {
    body.ldap_config = getLdapConfigFromForm();
  }
  if (selected.value === "oidc") {
    body.oidc_config = getOidcConfigFromForm();
  }
  try {
    const res = await apiCall("/admin/auth-config", { method: "PUT", body });
    if (res.ok) {
      alert("Authentication settings saved.");
    } else {
      alert("Failed to save: " + (await errorMessage(res)));
    }
  } catch (e) {
    alert("Error: " + e.message);
  }
}

// The 400 of the connection test: the message, and the network diagnosis
// lines the service appends when the directory was unreachable.
async function connectionTestFailure(res) {
  const raw = await res.text();
  if (!raw) return { message: `Request failed (${res.status})`, diagnostics: [] };
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.error === "string") {
      const diagnostics = Array.isArray(parsed.diagnostics)
        ? parsed.diagnostics.filter((line) => typeof line === "string")
        : [];
      return { message: parsed.error, diagnostics };
    }
  } catch (_) {}
  return { message: raw, diagnostics: [] };
}

async function testLdapConnection() {
  const resultEl = document.getElementById("ldap-test-result");
  const troubleEl = document.getElementById("ldap-troubleshooting");
  resultEl.classList.remove("hidden", "bg-green-50", "text-green-700", "bg-red-50", "text-red-700");
  troubleEl.classList.add("hidden");
  resultEl.textContent = "Testing…";
  resultEl.classList.add("bg-slate-50", "text-slate-600");
  try {
    const res = await apiCall("/admin/auth-config/test", {
      method: "POST",
      body: getLdapConfigFromForm(),
    });
    resultEl.classList.remove("bg-slate-50", "text-slate-600");
    if (res.ok) {
      resultEl.textContent = "✓ Connection successful";
      resultEl.classList.add("bg-green-50", "text-green-700");
    } else {
      const { message, diagnostics } = await connectionTestFailure(res);
      resultEl.textContent = "✗ Connection failed: " + message;
      if (diagnostics.length) {
        // The container's own view of the network, one line each; kept
        // monospace so the per-address columns line up.
        const pre = document.createElement("pre");
        pre.className = "mt-2 font-mono text-xs whitespace-pre-wrap";
        pre.textContent = diagnostics.join("\n");
        resultEl.appendChild(pre);
      }
      resultEl.classList.add("bg-red-50", "text-red-700");
      troubleEl.classList.remove("hidden");
    }
  } catch (e) {
    resultEl.classList.remove("bg-slate-50", "text-slate-600");
    resultEl.textContent = "✗ " + e.message;
    resultEl.classList.add("bg-red-50", "text-red-700");
    troubleEl.classList.remove("hidden");
  }
}

// --- Change Password ---
function changePassword() {
  document.getElementById("password-modal").classList.remove("hidden");
  document.getElementById("password-error").classList.add("hidden");
  document.getElementById("password-success").classList.add("hidden");
  document.getElementById("password-form").reset();
}

function closePasswordModal() {
  document.getElementById("password-modal").classList.add("hidden");
}

// Uses fetch directly instead of apiCall so that a 401 (wrong current
// password) is shown as a field error rather than triggering the global
// "session expired" logout flow.
document.getElementById("password-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const errEl = document.getElementById("password-error");
  const successEl = document.getElementById("password-success");
  errEl.classList.add("hidden");
  successEl.classList.add("hidden");
  try {
    if (!csrfToken) await fetchCsrfToken();
    const headers = { "Content-Type": "application/json" };
    if (sanshainToken) headers["Authorization"] = `Bearer ${sanshainToken}`;
    if (csrfToken) headers["X-CSRF-Token"] = csrfToken;
    const res = await fetch("/auth/change-password", {
      method: "POST",
      headers,
      body: JSON.stringify({
        old_password: document.getElementById("current-password").value,
        new_password: document.getElementById("new-password").value,
      }),
    });
    if (res.ok) {
      const data = await res.json();
      if (data && data.token) {
        sanshainToken = data.token;
        localStorage.setItem("sanshain_token", sanshainToken);
      }
      successEl.textContent = "Password updated successfully.";
      successEl.classList.remove("hidden");
      setTimeout(closePasswordModal, 1500);
    } else if (res.status === 401) {
      errEl.textContent = "Current password is incorrect.";
      errEl.classList.remove("hidden");
    } else {
      errEl.textContent = `Error: ${res.status}`;
      errEl.classList.remove("hidden");
    }
  } catch (err) {
    errEl.textContent = friendlyError(err);
    errEl.classList.remove("hidden");
  }
});

// --- Nuke Modal ---
let nukeCallback = null;
let nukeExpectedText = "";

function nukeConfirm(message, expectedText, callback) {
  document.getElementById("nuke-message").innerHTML = message;
  document.getElementById("nuke-expected-text").textContent = expectedText;
  document.getElementById("nuke-input").value = "";
  document.getElementById("nuke-error").classList.add("hidden");
  document.getElementById("nuke-yes").disabled = true;
  nukeExpectedText = expectedText;
  nukeCallback = callback;
  document.getElementById("nuke-modal").classList.remove("hidden");
  document.getElementById("nuke-input").focus();
}

document.getElementById("nuke-input").addEventListener("input", function () {
  document.getElementById("nuke-yes").disabled = this.value !== nukeExpectedText;
});

document.getElementById("nuke-yes").addEventListener("click", async function () {
  const input = document.getElementById("nuke-input").value;
  if (input !== nukeExpectedText) return;
  if (nukeCallback) {
    try {
      await nukeCallback();
      closeNukeModal();
    } catch (e) {
      const errEl = document.getElementById("nuke-error");
      errEl.textContent = "Operation failed: " + e.message;
      errEl.classList.remove("hidden");
    }
  }
});

function closeNukeModal() {
  document.getElementById("nuke-modal").classList.add("hidden");
  nukeCallback = null;
}

async function nukeServices() {
  const res = await apiCall("/admin/nuke/producers", {
    method: "POST",
    body: { confirmation: "DELETE ALL SERVICES" },
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  loadServices();
}

async function nukeClients() {
  const res = await apiCall("/admin/nuke/consumers", {
    method: "POST",
    body: { confirmation: "DELETE ALL CLIENTS" },
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  loadClients();
}

async function nukeUsers() {
  const res = await apiCall("/admin/nuke/users", {
    method: "POST",
    body: { confirmation: "DELETE ALL USERS" },
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  loadUsers();
}

async function nukeDatabase() {
  const res = await apiCall("/admin/nuke/database", {
    method: "POST",
    body: { confirmation: "NUKE DATABASE" },
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  loadAll();
}

// Confirmation wrappers for the destructive-operations buttons. Each button
// declares data-click to one of these instead of building a nukeConfirm()
// call (with its confirm-callback closure) inline.
function confirmNukeUsers() {
  nukeConfirm("☠️ Delete ALL non-admin users?", "DELETE ALL USERS", () => nukeUsers());
}
function confirmNukeServices() {
  nukeConfirm("☠️ Delete ALL services and their data?", "DELETE ALL SERVICES", () =>
    nukeServices(),
  );
}
function confirmNukeClients() {
  nukeConfirm("☠️ Delete ALL clients and their dependencies?", "DELETE ALL CLIENTS", () =>
    nukeClients(),
  );
}
function confirmNukeDatabase() {
  nukeConfirm(
    "💀☠️💀 NUKE THE ENTIRE DATABASE? This deletes ALL services, clients, users, versions, endpoints, and dependencies. Only your admin account will survive.",
    "NUKE DATABASE",
    () => nukeDatabase(),
  );
}

// Confirm-then-delete wrappers: each delete button declares data-click to one
// of these (with the entity name/id in data-click-args) instead of building a
// confirmDelete() call with a delete-callback closure inline.
function confirmDeleteUser(username, id) {
  confirmDelete(`Delete user <strong>${escapeHtml(username)}</strong>?`, () => deleteUser(id));
}
function confirmDeleteGroup(name, id) {
  confirmDelete(`Delete group <strong>${escapeHtml(name)}</strong>?`, () => deleteGroup(id));
}
function confirmDeleteService(svc) {
  confirmDelete(`Delete service <strong>${escapeHtml(svc)}</strong> and all its data?`, () =>
    deleteService(svc),
  );
}
function confirmDeleteClient(client) {
  confirmDelete(
    `Delete client <strong>${escapeHtml(client)}</strong> and all its dependencies?`,
    () => deleteClient(client),
  );
}

// --- Metadata Modal ---
function openMetadataModal(svc, icon, domain) {
  document.getElementById("metadata-service-name").textContent = svc;
  document.getElementById("metadata-svc-hidden").value = svc;
  document.getElementById("metadata-icon").value = icon || "";
  document.getElementById("metadata-domain").value = domain || "";
  document.getElementById("metadata-modal").classList.remove("hidden");
}

function closeMetadataModal() {
  document.getElementById("metadata-modal").classList.add("hidden");
}

document.getElementById("metadata-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const name = document.getElementById("metadata-svc-hidden").value;
  const icon = document.getElementById("metadata-icon").value.trim() || null;
  const domain = document.getElementById("metadata-domain").value.trim() || null;

  try {
    const res = await apiCall("/admin/producers/metadata", {
      method: "POST",
      body: { name, icon, domain },
    });
    if (res.ok) {
      closeMetadataModal();
      loadServices();
    } else {
      alert("Failed to save metadata: " + res.status);
    }
  } catch (err) {
    alert("Error: " + err.message);
  }
});

// --- Init ---
fetchCsrfToken().then(() => checkSession());
