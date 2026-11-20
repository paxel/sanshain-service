// V8 coverage of the page scripts, collected only when UI_COVERAGE is set (the
// UI job of CI sets it). It is part `ui` of the coverage ratchet
// (coverage-floors.toml) and what SonarQube Cloud reads. The fixture in
// fixtures.js adds each test's coverage; coverage-setup.js and
// coverage-teardown.js clear and write the report, coverage/ui.lcov, with
// paths relative to the repository root (static/js/...).
export const uiCoverageEnabled = Boolean(process.env.UI_COVERAGE);

// The report generator clears its output directory, so it gets one of its own
// and coverage-teardown.js copies the report next to the other parts.
export const uiCoverageReport = "coverage/ui/lcov.info";
export const uiCoverageLcov = "coverage/ui.lcov";

/** @type {import("monocart-coverage-reports").CoverageReportOptions} */
export const uiCoverageOptions = {
  name: "Sanshain UI tests",
  outputDir: "coverage/ui",
  reports: ["lcovonly"],
  // The service's own page scripts, and the documentation site's script that
  // site_stepper.test.js loads from the built site; the vendored libraries are not ours.
  entryFilter: (entry) =>
    (/\/js\/[\w/.-]+\.js(\?|$)/.test(entry.url) && !entry.url.includes("/vendor/")) ||
    /\/assets\/site\.js(\?|$)/.test(entry.url),
  // Served under /js/, kept in the repository under static/js/; the site's script is
  // copied unchanged from site/assets/.
  sourcePath: (filePath) => {
    if (/(^|\/)assets\/site\.js$/.test(filePath)) return "site/assets/site.js";
    const at = filePath.indexOf("js/");
    return at >= 0 ? `static/${filePath.slice(at)}` : filePath;
  },
};
