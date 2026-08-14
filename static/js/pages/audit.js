// Extracted from static/audit.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

function initAudit() {
  checkDiscoveryAuth(() => loadTimeline());
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initAudit);
} else {
  initAudit();
}
window.addEventListener("sanshain-update", () => loadTimeline());
