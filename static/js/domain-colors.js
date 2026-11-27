// Domain colours (san-1036), shared by the graph and the statistics page:
// each domain takes one from this palette by a hash of its name, so it keeps
// its colour across views, filters, links and pages.

// Purple, red, orange and amber are left out (they mark circular,
// conflicting, missing, stale and dangling edges); two domains may share a
// colour, the name beside it tells them apart. `border` is the strong tone, `text` a darker shade.
const GRAPH_DOMAIN_PALETTE = [
  { border: "#0d9488", text: "#115e59" }, // teal
  { border: "#0891b2", text: "#155e75" }, // cyan
  { border: "#0284c7", text: "#075985" }, // sky
  { border: "#2563eb", text: "#1e40af" }, // blue
  { border: "#4f46e5", text: "#3730a3" }, // indigo
  { border: "#059669", text: "#065f46" }, // emerald
  { border: "#65a30d", text: "#3f6212" }, // lime
  { border: "#475569", text: "#1e293b" }, // slate
  { border: "#57534e", text: "#292524" }, // stone
];

// FNV-1a over the name's UTF-16 code units: stable, and cheap enough to run
// per node on every redraw.
/** @param {string} domain */
function graphDomainColor(domain) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < domain.length; i++) {
    hash ^= domain.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  const { border, text } = /** @type {{ border: string, text: string }} */ (
    GRAPH_DOMAIN_PALETTE[hash % GRAPH_DOMAIN_PALETTE.length]
  );
  return { border, text, fill: `${border}14` };
}
window.graphDomainColor = graphDomainColor;
window.GRAPH_DOMAIN_PALETTE = GRAPH_DOMAIN_PALETTE;
