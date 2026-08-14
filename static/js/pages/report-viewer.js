// Extracted from static/report-viewer.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

let rawMarkdown = "";
let reportFilename = "report.md";

async function loadReport() {
  const params = new URLSearchParams(window.location.search);
  const src = params.get("src");
  const title = params.get("title") || "Report";

  document.getElementById("report-title").textContent = title;
  document.title = title + " — Sanshain";

  if (!src) {
    showError("No report source specified.");
    return;
  }

  // Derive filename from src
  const srcPath = src.split("?")[0].split("/").pop() || "report";
  reportFilename = `${srcPath}.md`;

  try {
    const headers = {};
    const token = localStorage.getItem("sanshain_token");
    if (token) headers["Authorization"] = `Bearer ${token}`;
    const resp = await fetch(src, { headers });
    if (!resp.ok) {
      if (resp.status === 401 || resp.status === 403) {
        showError("Authentication required. Please log in first.");
      } else {
        showError(`Server returned ${resp.status} ${resp.statusText}`);
      }
      return;
    }
    rawMarkdown = await resp.text();
    document.getElementById("loading").classList.add("hidden");
    const content = document.getElementById("report-content");
    let html = marked.parse(rawMarkdown);
    // Wrap tables in a scrollable container for long service names
    html = html
      .replace(/<table>/g, '<div class="table-wrapper"><table>')
      .replace(/<\/table>/g, "</table></div>");
    content.innerHTML = html;
    content.classList.remove("hidden");
  } catch (e) {
    showError(e.message);
  }
}

function showError(msg) {
  document.getElementById("loading").classList.add("hidden");
  document.getElementById("error").classList.remove("hidden");
  document.getElementById("error-detail").textContent = msg;
}

function copyMarkdown() {
  if (!rawMarkdown) return;
  const btn = document.getElementById("copy-btn");
  navigator.clipboard.writeText(rawMarkdown).then(() => {
    btn.innerHTML = `<svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg> Copied!`;
    setTimeout(() => {
      btn.innerHTML = `<svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2" ry="2" stroke-width="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" stroke-width="2"/></svg> Copy Markdown`;
    }, 2000);
  });
}

function downloadMarkdown() {
  if (!rawMarkdown) return;
  const blob = new Blob([rawMarkdown], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = reportFilename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

loadReport();
