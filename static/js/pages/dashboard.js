// Extracted from templates/dashboard.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

if (!sanshainToken) {
  window.location.href = "/";
}

async function loadTokens() {
  try {
    const res = await fetch("/auth/tokens", {
      headers: { Authorization: "Bearer " + sanshainToken },
    });
    if (res.status === 401) {
      localStorage.removeItem("sanshain_token");
      window.location.href = "/index.html";
      return;
    }
    const tokens = await res.json();
    const tbody = document.getElementById("tokenTableBody");
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
    document.getElementById("dashboard-main").classList.remove("hidden");
  } catch (_) {
    hideLoader();
  }
}

document.getElementById("createTokenForm").addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!csrfToken) await fetchCsrfToken();
  const name = document.getElementById("tokenName").value;
  const expires_in_days = parseInt(document.getElementById("tokenExpiry").value);
  const res = await fetch("/auth/tokens", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: "Bearer " + sanshainToken,
      "X-CSRF-Token": csrfToken,
    },
    body: JSON.stringify({ name, expires_in_days }),
  });
  csrfToken = null;
  if (!res.ok) {
    const text = await res.text();
    alert("Failed to create token: " + (text || res.statusText));
    return;
  }
  const data = await res.json();
  document.getElementById("tokenValue").textContent = data.token;
  document.getElementById("tokenValueUsage").textContent = data.token;
  document.getElementById("tokenModal").classList.remove("hidden");
  document.getElementById("tokenName").value = "";
  loadTokens();
});

document.getElementById("copyTokenBtn").addEventListener("click", () => {
  const token = document.getElementById("tokenValue").textContent;
  navigator.clipboard.writeText(token).then(() => {
    document.getElementById("copyTokenBtn").textContent = "✅ Copied!";
    setTimeout(() => {
      document.getElementById("copyTokenBtn").textContent = "📋 Copy to Clipboard";
    }, 2000);
  });
});

document.getElementById("closeModal").addEventListener("click", () => {
  document.getElementById("tokenModal").classList.add("hidden");
});

async function revokeToken(id) {
  if (!confirm("Revoke this token? This cannot be undone.")) return;
  if (!csrfToken) await fetchCsrfToken();
  await fetch("/auth/tokens/" + id, {
    method: "DELETE",
    headers: {
      Authorization: "Bearer " + sanshainToken,
      "X-CSRF-Token": csrfToken,
    },
  });
  csrfToken = null;
  loadTokens();
}

fetchCsrfToken();
renderBanner();
loadTokens();
