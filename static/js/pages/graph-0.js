// Extracted from static/graph.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

mermaid.initialize({
  startOnLoad: false,
  theme: "base",
  themeVariables: { fontSize: "14px", fontFamily: "ui-sans-serif, system-ui, sans-serif" },
});
