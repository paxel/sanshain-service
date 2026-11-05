// The site's header (banner and navigation) and footer, written once for every
// page. A page holds empty placeholders and loads this script right after
// each, so the markup is in place before the rest of the page is parsed and
// nothing jumps:
//
//   <header id="site-banner"></header><script src="/js/site-chrome.js"></script>
//   <footer id="site-footer"></footer><script src="/js/site-chrome.js"></script>
//
// The first run also puts up the loading screen (common.js's injectLoader),
// which the page hides once its data is in. Links whose destination needs a
// permission start hidden; common.js's updateBannerAuth shows them, and
// highlightCurrentBannerLink marks the page's own.
(function renderSiteChrome() {
  const banner = document.getElementById("site-banner");
  if (banner && !banner.childElementCount) {
    banner.className = "bg-indigo-700 text-white p-4 shadow-lg";
    const link = (href, label, extra = "") =>
      `<a href="${href}" ${extra} class="text-white hover:text-indigo-100 transition-colors">${label}</a>`;
    const gated = (id, href, label) =>
      `<a id="${id}" href="${href}" class="hidden text-white hover:text-indigo-100 transition-colors">${label}</a>`;
    banner.innerHTML = `
      <div class="container mx-auto flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div class="flex items-center justify-between gap-4 md:justify-start">
          <a href="/" class="flex items-center space-x-3">
            <img src="/images/sonne.png" alt="Sanshain Logo" width="66" height="36"
              class="h-9 w-auto rounded-md bg-white/10 p-0.5" />
            <div class="text-2xl font-bold tracking-tight">
              Sanshain <span class="text-indigo-200">(サンシャイン)</span>
            </div>
          </a>
          <span id="version-badge" class="text-indigo-200 text-sm md:hidden"></span>
        </div>
        <nav class="flex flex-wrap items-center justify-center gap-x-4 gap-y-2 text-sm font-medium md:flex-1">
          ${link("/producers.html", "Producers")}
          ${link("/consumers.html", "Consumers")}
          ${link("/graph.html", "Graph")}
          ${link("/reports.html", "Reports")}
          ${gated("nav-audit-link", "/audit.html", "Audit")}
          ${link("/observability.html", "Observability")}
          ${gated("nav-admin-link", "/admin.html", "Admin")}
        </nav>
        <div class="flex items-center justify-end gap-4 text-sm">
          <span id="version-badge-desktop" class="text-indigo-200 max-md:hidden"></span>
          <a id="banner-username" href="/account.html"
            class="hidden text-indigo-200 hover:text-white transition-colors cursor-pointer"></a>
          <button id="banner-logout" type="button" data-click="sanshainLogout"
            class="hidden rounded-lg bg-indigo-600 px-3 py-1 font-medium transition-colors hover:bg-indigo-500">
            Logout
          </button>
          <a id="banner-signin" href="/account.html"
            class="hidden text-indigo-100 transition-colors hover:text-white">Sign In</a>
        </div>
      </div>`;
  }

  const footer = document.getElementById("site-footer");
  if (footer && !footer.childElementCount) {
    footer.className = "text-center text-sm text-slate-500 py-6 mt-auto";
    footer.innerHTML = `
      <img src="/images/sonne.png" alt="Sanshain" width="59" height="32"
        class="mx-auto mb-2 h-8 w-auto opacity-70" />
      <div>Sanshain (サンシャイン) &mdash; OpenAPI Distribution Service</div>
      <div class="mt-2 flex items-center justify-center gap-4">
        <a href="https://github.com/paxel/sanshain-service" target="_blank" rel="noopener noreferrer"
          class="text-indigo-600 hover:text-indigo-800">GitHub</a>
        <a href="/LICENSE" class="text-indigo-600 hover:text-indigo-800">License</a>
      </div>`;
  }

  if (typeof injectLoader === "function") injectLoader();
})();
