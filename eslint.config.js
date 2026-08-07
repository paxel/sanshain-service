import globals from "globals";
import eslintConfigPrettier from "eslint-config-prettier";

export default [
  {
    files: ["static/js/*.js"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "script",
      globals: {
        ...globals.browser,
        // External libraries loaded via <script> tags
        dagre: "readonly",
        Diff2Html: "readonly",
        // Cross-file globals (common.js exports used by other files)
        renderBanner: "readonly",
        sanshainLogout: "readonly",
        friendlyError: "readonly",
        errorMessage: "readonly",
        escapeHtml: "readonly",
        escapeAttr: "readonly",
        confirmDelete: "readonly",
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
