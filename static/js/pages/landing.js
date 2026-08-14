// Extracted from templates/index.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

hideLoader();
renderBanner();

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
