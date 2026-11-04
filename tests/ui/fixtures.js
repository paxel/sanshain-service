// The `test` every UI test imports: Playwright's, plus an automatic fixture
// that records the page's script coverage when UI_COVERAGE is set.
import { test as base, expect } from "@playwright/test";
import MCR from "monocart-coverage-reports";
import { uiCoverageEnabled, uiCoverageOptions } from "./coverage.js";

export const test = base.extend({
  recordCoverage: [
    async ({ page, browserName }, use) => {
      const record = uiCoverageEnabled && browserName === "chromium";
      if (record) await page.coverage.startJSCoverage({ resetOnNavigation: false });
      await use();
      if (!record) return;
      const entries = await page.coverage.stopJSCoverage();
      // A test that loaded no script has nothing to add.
      if (entries.length > 0) await MCR(uiCoverageOptions).add(entries);
    },
    { scope: "test", auto: true },
  ],
});

export { expect };
