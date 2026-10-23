// Extracted from static/account.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// Use sanshainToken from common.js
function onSessionExpired() {
  showAuth();
}

// --- Tab switching ---
function showTab(tab) {
  const loginTab = document.getElementById("tab-login");
  const registerTab = document.getElementById("tab-register");
  const loginPanel = document.getElementById("login-panel");
  const registerPanel = document.getElementById("register-panel");
  if (tab === "login") {
    loginTab.classList.add("border-indigo-600", "text-indigo-700");
    loginTab.classList.remove("border-transparent", "text-slate-500");
    registerTab.classList.remove("border-indigo-600", "text-indigo-700");
    registerTab.classList.add("border-transparent", "text-slate-500");
    loginPanel.classList.remove("hidden");
    registerPanel.classList.add("hidden");
  } else {
    registerTab.classList.add("border-indigo-600", "text-indigo-700");
    registerTab.classList.remove("border-transparent", "text-slate-500");
    loginTab.classList.remove("border-indigo-600", "text-indigo-700");
    loginTab.classList.add("border-transparent", "text-slate-500");
    registerPanel.classList.remove("hidden");
    loginPanel.classList.add("hidden");
  }
}

// --- Screen switching ---
function showAuth() {
  hideLoader();
  document.getElementById("auth-screen").classList.remove("hidden");
  document.getElementById("account-dashboard").classList.add("hidden");
  applyLoginOptions();
}

// What the sign-in card offers follows the instance's settings: the register
// tab only while registration is open, the SSO button only while OIDC is the
// source, and a notice while only administrators can sign in.
let loginOptions = { registration: false, auto_approve: false, sso: false };

async function applyLoginOptions() {
  try {
    const res = await fetch("/auth/options");
    if (!res.ok) return;
    loginOptions = await res.json();
  } catch (_) {
    return;
  }
  document.getElementById("tab-register").classList.toggle("hidden", !loginOptions.registration);
  if (!loginOptions.registration) showTab("login");
  document.getElementById("sso-login").classList.toggle("hidden", !loginOptions.sso);
  document.getElementById("register-help").textContent = loginOptions.auto_approve
    ? "You can sign in as soon as your account is created."
    : "After registration an admin must approve your account before you can sign in.";
  const notice = document.getElementById("auth-notice");
  notice.textContent = !loginOptions.admins_only
    ? ""
    : loginOptions.auth_mode === "dev"
      ? "Sanshain is in dev mode. Only administrators can sign in."
      : "Sanshain is in maintenance. Only administrators can sign in.";
  notice.classList.toggle("hidden", !loginOptions.admins_only);
}

function showAccount() {
  hideLoader();
  document.getElementById("auth-screen").classList.add("hidden");
  document.getElementById("account-dashboard").classList.remove("hidden");
  loadTokens();
  loadServiceAccountChoices();
  showPasswordChangeForLocalSignIns();
}

// A session that signed in through LDAP or OIDC has no Sanshain password to
// change. A session from before the sign-in was recorded keeps the form.
// The display name is the person's own unless the provider supplies it.
async function showPasswordChangeForLocalSignIns() {
  try {
    const res = await apiCall("/auth/me");
    if (!res.ok) return;
    const me = await res.json();
    document
      .getElementById("change-password-section")
      .classList.toggle("hidden", Boolean(me.sign_in) && me.sign_in !== "local");
    const editable = me.display_name_editable !== false;
    document.getElementById("display-name-form").classList.toggle("hidden", !editable);
    document.getElementById("display-name-from-provider").classList.toggle("hidden", editable);
    document.getElementById("display-name-input").value = me.display_name || "";
    document.getElementById("display-name-value").textContent = me.display_name || me.username;
  } catch (_) {}
}

document.getElementById("display-name-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const status = document.getElementById("display-name-status");
  try {
    if (!csrfToken) await fetchCsrfToken();
    const res = await apiCall("/auth/display-name", {
      method: "PUT",
      body: { display_name: document.getElementById("display-name-input").value },
    });
    const ok = res.ok;
    status.textContent = ok ? "Saved." : await errorMessage(res);
    status.className = `text-sm mt-2 ${ok ? "text-green-600" : "text-red-600"}`;
    if (ok) await renderBanner();
  } catch (err) {
    status.textContent = friendlyError(err);
    status.className = "text-sm mt-2 text-red-600";
  }
});

// The "For" choice of the token form: the service accounts, for whoever may
// list users (manage_users); hidden for everyone else and when none exist.
async function loadServiceAccountChoices() {
  const row = document.getElementById("token-for-row");
  const select = document.getElementById("token-for");
  let accounts = [];
  try {
    const me = await apiCall("/auth/me");
    const user = me.ok ? await me.json() : null;
    if (user && (user.is_root || hasPermission(user, "manage_users"))) {
      const res = await apiCall("/admin/users");
      if (res.ok) accounts = (await res.json()).filter((u) => u.service_account);
    }
  } catch (_) {}
  select.replaceChildren(new Option("Me", ""));
  for (const account of accounts) select.add(new Option(personLabel(account), account.username));
  row.classList.toggle("hidden", accounts.length === 0);
}

// --- Login ---
document.getElementById("login-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const errEl = document.getElementById("login-error");
  errEl.classList.add("hidden");
  try {
    if (!csrfToken) await fetchCsrfToken();
    const res = await fetch("/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrfToken || "" },
      body: JSON.stringify({
        username: document.getElementById("login-username").value,
        password: document.getElementById("login-password").value,
      }),
    });
    if (!res.ok) {
      const message = res.status === 401 ? "" : await errorMessage(res);
      // A bare 403 is an account still waiting for approval; one with a
      // reason (maintenance, dev mode) says it itself.
      errEl.textContent =
        res.status === 401
          ? "Invalid username or password."
          : message === "Forbidden"
            ? "Your account is waiting for an admin's approval."
            : message;
      errEl.classList.remove("hidden");
      return;
    }
    const data = await res.json();
    sanshainToken = data.token;
    localStorage.setItem("sanshain_token", sanshainToken);
    // Signed in: on to the dashboard, like an OIDC sign-in.
    window.location.href = "/";
  } catch (err) {
    errEl.textContent = friendlyError(err);
    errEl.classList.remove("hidden");
  }
});

// --- Register ---
document.getElementById("register-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const errEl = document.getElementById("register-error");
  const successEl = document.getElementById("register-success");
  errEl.classList.add("hidden");
  successEl.classList.add("hidden");

  const password = document.getElementById("register-password").value;
  const confirm = document.getElementById("register-password-confirm").value;
  if (password !== confirm) {
    errEl.textContent = "Passwords do not match.";
    errEl.classList.remove("hidden");
    return;
  }

  try {
    if (!csrfToken) await fetchCsrfToken();
    const res = await fetch("/auth/register", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrfToken || "" },
      body: JSON.stringify({
        username: document.getElementById("register-username").value,
        password: password,
      }),
    });
    if (res.status === 201) {
      const { approved } = await res.json().catch(() => ({ approved: false }));
      successEl.textContent = approved
        ? "Account created. You can sign in now."
        : "Registration successful! An admin must approve your account before you can sign in.";
      successEl.classList.remove("hidden");
      document.getElementById("register-form").reset();
    } else if (res.status === 409) {
      errEl.textContent = "Username already taken.";
      errEl.classList.remove("hidden");
    } else if (res.status === 403) {
      errEl.textContent = "Registration is currently disabled.";
      errEl.classList.remove("hidden");
    } else {
      errEl.textContent = `Registration failed (${res.status}).`;
      errEl.classList.remove("hidden");
    }
  } catch (err) {
    errEl.textContent = friendlyError(err);
    errEl.classList.remove("hidden");
  }
});

// --- Logout ---
async function doLogout() {
  await sanshainLogout({ redirectTo: null });
  showAuth();
}

// --- Change Password ---
// Uses fetch directly instead of apiCall so that a 401 (wrong current
// password) is shown as a field error rather than triggering the global
// "session expired" logout flow.
document.getElementById("pw-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const errEl = document.getElementById("pw-error");
  const successEl = document.getElementById("pw-success");
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
        old_password: document.getElementById("pw-current").value,
        new_password: document.getElementById("pw-new").value,
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
      document.getElementById("pw-form").reset();
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

// --- Tokens ---
// The scope is decided at creation and immutable, so the list is the only
// place a user can tell a read-only token from a full one.
function tokenScopeBadge(scope) {
  if (scope === "read_only") {
    return '<span class="ml-2 text-xs bg-amber-100 text-amber-800 px-2 py-0.5 rounded-full" data-token-scope="read_only">read-only</span>';
  }
  return '<span class="ml-2 text-xs bg-slate-200 text-slate-700 px-2 py-0.5 rounded-full" data-token-scope="full">full</span>';
}

async function loadTokens() {
  const list = document.getElementById("tokens-list");
  try {
    const res = await apiCall("/auth/tokens");
    if (!res.ok) {
      list.innerHTML = '<p class="text-sm text-red-500">Failed to load tokens.</p>';
      return;
    }
    const tokens = await res.json();
    if (tokens.length === 0) {
      list.innerHTML = '<p class="text-sm text-slate-500 italic">No API tokens created yet.</p>';
      return;
    }
    list.innerHTML = tokens
      .map(
        (t) => `
                    <div class="flex justify-between items-center bg-slate-50 px-4 py-3 rounded-lg fade-in">
                        <div>
                            <span class="font-medium text-slate-800">${escapeHtml(t.name)}</span>
                            ${tokenScopeBadge(t.scope)}
                            ${t.for_user ? `<div class="text-xs text-slate-600 mt-0.5" data-token-for>For <strong>${escapeHtml(t.for_user)}</strong>${t.issued_by ? ` · issued by ${escapeHtml(t.issued_by)}` : ""}</div>` : ""}
                            <div class="text-xs text-slate-500 mt-0.5">
                                Created: ${escapeHtml(t.created_at)} · Expires: ${escapeHtml(t.expires_at)}
                                · Last used: ${t.last_used_at ? escapeHtml(t.last_used_at) : "never"}
                            </div>
                        </div>
                        <button data-click="confirmRevokeToken" data-click-args="${attrJson([t.name, t.id])}"
                            class="text-red-500 hover:text-red-700 text-sm">Revoke</button>
                    </div>
                `,
      )
      .join("");
  } catch (_) {
    list.innerHTML = '<p class="text-sm text-red-500">Failed to load tokens.</p>';
  }
}

document.getElementById("token-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const errEl = document.getElementById("token-create-error");
  const createdEl = document.getElementById("token-created");
  errEl.classList.add("hidden");
  createdEl.classList.add("hidden");
  try {
    const res = await apiCall("/auth/tokens", {
      method: "POST",
      body: {
        name: document.getElementById("token-name").value,
        expires_in_days: parseInt(document.getElementById("token-expires").value, 10),
        scope: document.getElementById("token-read-only").checked ? "read_only" : "full",
        for_user: document.getElementById("token-for").value || null,
      },
    });
    if (res.ok) {
      const data = await res.json();
      document.getElementById("token-created-value").textContent = data.token;
      createdEl.classList.remove("hidden");
      document.getElementById("token-form").reset();
      document.getElementById("token-expires").value = "365";
      loadTokens();
    } else {
      errEl.textContent = `Failed to create token: ${await errorMessage(res)}`;
      errEl.classList.remove("hidden");
    }
  } catch (err) {
    errEl.textContent = err.message;
    errEl.classList.remove("hidden");
  }
});

function copyToken() {
  const val = document.getElementById("token-created-value").textContent;
  navigator.clipboard.writeText(val).catch(() => {});
}

async function revokeToken(id) {
  try {
    await apiCall(`/auth/tokens/${encodeURIComponent(id)}`, { method: "DELETE" });
    loadTokens();
  } catch (_) {}
}

// Confirm-then-revoke wrapper: the token row's delete button declares
// data-click to this instead of building a confirmAction() call inline.
function confirmRevokeToken(name, id) {
  confirmAction(`Revoke token <strong>${escapeHtml(name)}</strong>?`, () => revokeToken(id));
}

// --- Check session on load ---
async function checkSession() {
  if (!sanshainToken) {
    showAuth();
    return;
  }
  try {
    const res = await apiCall("/auth/me");
    if (res.ok) {
      const data = await res.json();
      renderBanner(data);
      showAccount();
    } else {
      showAuth();
    }
  } catch (_) {
    showAuth();
  }
}

fetchCsrfToken().then(() => checkSession());
