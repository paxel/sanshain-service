import globals from "globals";
import eslintConfigPrettier from "eslint-config-prettier";

export default [
  {
    files: ["static/js/*.js", "static/js/pages/*.js"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "script",
      globals: {
        ...globals.browser,
        // External libraries loaded via <script> tags
        dagre: "readonly",
        Diff2Html: "readonly",
        mermaid: "readonly",
        marked: "readonly",
        // Cross-file globals (common.js exports used by other files)
        renderBanner: "readonly",
        sanshainLogout: "readonly",
        friendlyError: "readonly",
        errorMessage: "readonly",
        escapeHtml: "readonly",
        escapeAttr: "readonly",
        attrJson: "readonly",
        setActionArgs: "readonly",
        confirmDelete: "readonly",
        confirmAction: "readonly",
        togglePasswordVisibility: "readonly",
        onSessionExpired: "readonly",
        showLoader: "readonly",
        hideLoader: "readonly",
        apiCall: "readonly",
        getSanshainToken: "readonly",
        // discovery.js globals
        loadAllProducers: "readonly",
        getMethodColor: "readonly",
        renderPaginatedYaml: "readonly",
        checkDiscoveryAuth: "readonly",
        closeModal: "readonly",
        // Cross-file page globals surfaced when inline <script> blocks were
        // extracted to static/js/pages/ (ai/improvements.md #11). Each is
        // defined in another script loaded on the same page; runtime scope is
        // unchanged from when the code was inline.
        allServices: "readonly",
        canSeeAdminDashboard: "readonly",
        compareSemver: "readonly",
        csrfToken: "readonly",
        exportToPng: "readonly",
        fetchCsrfToken: "readonly",
        fetchJSON: "readonly",
        graphEdgeFlags: "readonly",
        graphLatestGaMap: "readonly",
        hasPermission: "readonly",
        loadTimeline: "readonly",
        loadUserFavorites: "readonly",
        userFavorites: "writable",
        producerHasAnyEndpoints: "readonly",
        promoteVersion: "readonly",
        redrawGraph: "readonly",
        renderFocusTags: "readonly",
        renderUnifiedDiffHtml: "readonly",
        sanshainToken: "readonly",
        simpleDiff: "readonly",
        stabilityBadge: "readonly",
        toggleFavorite: "readonly",
        updateHighlightButtons: "readonly",
        // graph.js globals
        lastGraphReport: "writable",
        currentGraphMode: "writable",
        currentGraphDirection: "writable",
        renderGraph: "readonly",
        getCustomGraphSVG: "readonly",
      },
    },
    rules: {
      "no-unused-vars": "off",
      "no-undef": "error",
    },
  },
  eslintConfigPrettier,
];
