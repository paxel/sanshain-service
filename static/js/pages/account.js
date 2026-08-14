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
}

function showAccount() {
  hideLoader();
  document.getElementById("auth-screen").classList.add("hidden");
  document.getElementById("account-dashboard").classList.remove("hidden");
  loadTokens();
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
      errEl.textContent =
        res.status === 401 ? "Invalid credentials or account not approved" : `Error: ${res.status}`;
      errEl.classList.remove("hidden");
      return;
    }
    const data = await res.json();
    sanshainToken = data.token;
    localStorage.setItem("sanshain_token", sanshainToken);
    await renderBanner();
    showAccount();
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
      successEl.textContent =
        "Registration successful! An admin must approve your account before you can log in.";
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
                            <div class="text-xs text-slate-500 mt-0.5">
                                Created: ${escapeHtml(t.created_at)} · Expires: ${escapeHtml(t.expires_at)}
                                ${t.last_used_at ? " · Last used: " + escapeHtml(t.last_used_at) : ""}
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
      errEl.textContent = `Failed to create token (${res.status}).`;
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
