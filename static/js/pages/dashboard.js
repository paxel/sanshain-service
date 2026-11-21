// Extracted from templates/dashboard.html: inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// What the token rows name in their data-click attributes.
/* exported revokeToken */

/**
 * `ApiToken` in `src/domain/models.rs`, as `GET /auth/tokens` lists it.
 * @typedef {object} DashboardToken
 * @property {string} id
 * @property {string} name
 * @property {string} created_at
 * @property {string} expires_at
 * @property {string | null} [last_used_at]
 */

if (!sanshainToken) {
  window.location.href = "/";
}

async function loadTokens() {
  try {
    const res = await fetch("/auth/tokens", {
      headers: { Authorization: "Bearer " + String(sanshainToken) },
    });
    if (res.status === 401) {
      localStorage.removeItem("sanshain_token");
      window.location.href = "/index.html";
      return;
    }
    const tokens = /** @type {DashboardToken[]} */ (await res.json());
    const tbody = /** @type {HTMLElement} */ (document.getElementById("tokenTableBody"));
    if (tokens.length === 0) {
      tbody.innerHTML =
        '<tr><td colspan="5" class="py-4 text-slate-400 text-center">No tokens yet. Create one above.</td></tr>';
    } else {
      tbody.innerHTML = tokens
        .map(
          (t) => `
                <tr class="border-b border-slate-100">
                    <td class="py-2 pr-4 font-medium">${escapeHtml(t.name)}</td>
                    <td class="py-2 pr-4 text-slate-500">${escapeHtml(t.created_at)}</td>
                    <td class="py-2 pr-4 text-slate-500">${escapeHtml(t.expires_at)}</td>
                    <td class="py-2 pr-4 text-slate-500">${t.last_used_at ? escapeHtml(t.last_used_at) : '<span class="italic">Never</span>'}</td>
                    <td class="py-2">
                        <button data-click="revokeToken" data-click-args="${attrJson([t.id])}" class="text-red-600 hover:text-red-800 text-sm font-medium">🗑 Revoke</button>
                    </td>
                </tr>
            `,
        )
        .join("");
    }
    // Reveal dashboard after tokens loaded
    hideLoader();
    /** @type {HTMLElement} */ (document.getElementById("dashboard-main")).classList.remove(
      "hidden",
    );
  } catch {
    hideLoader();
  }
}

/** @type {HTMLElement} */ (document.getElementById("createTokenForm")).addEventListener(
  "submit",
  (e) => {
    void (async () => {
      e.preventDefault();
      if (!csrfToken) await fetchCsrfToken();
      const name = /** @type {HTMLInputElement} */ (document.getElementById("tokenName")).value;
      const expires_in_days = Number.parseInt(
        /** @type {HTMLSelectElement} */ (document.getElementById("tokenExpiry")).value,
      );
      const res = await fetch("/auth/tokens", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: "Bearer " + String(sanshainToken),
          // Without a token the header says "null", and the server refuses the call.
          "X-CSRF-Token": /** @type {string} */ (csrfToken),
        },
        body: JSON.stringify({ name, expires_in_days }),
      });
      forgetCsrfToken();
      if (!res.ok) {
        const text = await res.text();
        alert("Failed to create token: " + (text || res.statusText));
        return;
      }
      const data = /** @type {{ token: string }} */ (await res.json());
      /** @type {HTMLElement} */ (document.getElementById("tokenValue")).textContent = data.token;
      /** @type {HTMLElement} */ (document.getElementById("tokenValueUsage")).textContent =
        data.token;
      /** @type {HTMLElement} */ (document.getElementById("tokenModal")).classList.remove("hidden");
      /** @type {HTMLInputElement} */ (document.getElementById("tokenName")).value = "";
      void loadTokens();
    })();
  },
);

/** @type {HTMLElement} */ (document.getElementById("copyTokenBtn")).addEventListener(
  "click",
  () => {
    const token = /** @type {HTMLElement} */ (document.getElementById("tokenValue")).textContent;
    void navigator.clipboard.writeText(/** @type {string} */ (token)).then(() => {
      /** @type {HTMLElement} */ (document.getElementById("copyTokenBtn")).textContent =
        "✅ Copied!";
      setTimeout(() => {
        /** @type {HTMLElement} */ (document.getElementById("copyTokenBtn")).textContent =
          "📋 Copy to Clipboard";
      }, 2000);
    });
  },
);

/** @type {HTMLElement} */ (document.getElementById("closeModal")).addEventListener("click", () => {
  /** @type {HTMLElement} */ (document.getElementById("tokenModal")).classList.add("hidden");
});

/** @param {string} id */
async function revokeToken(id) {
  if (!confirm("Revoke this token? This cannot be undone.")) return;
  if (!csrfToken) await fetchCsrfToken();
  await fetch("/auth/tokens/" + id, {
    method: "DELETE",
    headers: {
      Authorization: "Bearer " + String(sanshainToken),
      // Without a token the header says "null", and the server refuses the call.
      "X-CSRF-Token": /** @type {string} */ (csrfToken),
    },
  });
  forgetCsrfToken();
  void loadTokens();
}

void fetchCsrfToken();
void renderBanner();
void loadTokens();
