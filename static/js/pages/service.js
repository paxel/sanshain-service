// Extracted from static/service.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// Backward-compatibility redirect: map old hash-based URLs to new standalone pages
const hash = window.location.hash.replace(/^#/, "").toLowerCase();
const map = {
  clients: "/consumers.html",
  graph: "/graph.html",
  reports: "/reports.html",
};
window.location.replace(map[hash] || "/producers.html");
