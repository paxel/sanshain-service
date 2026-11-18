// The strict TypeScript lint config of the standard (paxel/gates configs/ts): typescript-eslint's
// strictTypeChecked and stylisticTypeChecked with type information, over JavaScript that
// tsconfig.json type-checks (allowJs, checkJs). There is no build step: the page scripts are
// served as they are written.
import eslint from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import tseslint from "typescript-eslint";
import { defineConfig } from "eslint/config";

export default defineConfig(
  {
    // Third-party bundles, generated files and build output are not ours to lint.
    ignores: [
      "node_modules/",
      "target/",
      "coverage/",
      "build/",
      "site-dist/",
      "playwright-report/",
      "test-results/",
      "static/vendor/",
      "static/css/",
      "fuzz/",
      "deploy/",
    ],
  },
  {
    // Temporary: the groups san-1003 has not cleaned yet. Its last commit removes this block.
    ignores: [
      "static/js/pages/",
      "static/js/graph.js",
      "static/js/graph-domains.js",
      "static/js/webhooks.js",
      "static/js/audit.js",
      "static/js/structure.js",
      "static/js/endpoint-filter.js",
      "tests/ui/",
      "site/assets/",
    ],
  },
  eslint.configs.recommended,
  tseslint.configs.strictTypeChecked,
  tseslint.configs.stylisticTypeChecked,
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // The standard's config turns this off for TypeScript files, where the compiler reports
      // an unknown name. With checkJs the compiler does the same for these JavaScript files
      // (TS2304), from the declarations in types/ instead of a second list of globals here.
      "no-undef": "off",
    },
  },
  {
    // The page scripts and the documentation site's script are classic scripts: what one
    // declares at the top level is a global for the scripts loaded after it on the same page.
    files: ["static/js/**/*.js", "site/assets/*.js"],
    languageOptions: { sourceType: "script" },
  },
  eslintConfigPrettier,
);
