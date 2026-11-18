// The `test` every UI test imports: Playwright's, plus an automatic fixture
// that records the page's script coverage when UI_COVERAGE is set.
import { test as base, expect } from "@playwright/test";
import MCR from "monocart-coverage-reports";
import { uiCoverageEnabled, uiCoverageOptions } from "./coverage.js";

/**
 * @typedef {import("@playwright/test").PlaywrightTestArgs &
 *   import("@playwright/test").PlaywrightTestOptions} TestArgs
 */
/**
 * @typedef {import("@playwright/test").PlaywrightWorkerArgs &
 *   import("@playwright/test").PlaywrightWorkerOptions} WorkerArgs
 */
/**
 * @typedef {import("@playwright/test").Fixtures<
 *   { recordCoverage: void },
 *   object,
 *   TestArgs,
 *   WorkerArgs
 * >} CoverageFixtures
 */
/**
 * `extend` cannot infer the fixture's name from the object, and JavaScript cannot pass it the
 * type argument: this is `extend<{ recordCoverage: void }>` written out.
 * @typedef {object} CoverageExtend
 * @property {(
 *   fixtures: CoverageFixtures,
 * ) => import("@playwright/test").TestType<TestArgs & { recordCoverage: void }, WorkerArgs>} extend
 */

/**
 * Returns the JSON body of an API response. Playwright types it as `any`: read as
 * `unknown`, the test says which shape it expects with a cast.
 * @param {import("@playwright/test").APIResponse} response
 * @returns {Promise<unknown>}
 */
export const json = (response) => response.json();

export const test = /** @type {CoverageExtend} */ (base).extend({
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
