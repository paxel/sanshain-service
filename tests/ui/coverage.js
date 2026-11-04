// V8 coverage of the page scripts for SonarQube Cloud, collected only when
// UI_COVERAGE is set (Quality's UI job sets it). The fixture in fixtures.js
// adds each test's coverage; coverage-setup.js and coverage-teardown.js clear
// and write the report, build/ui-coverage/lcov.info.
export const uiCoverageEnabled = Boolean(process.env.UI_COVERAGE);

export const uiCoverageOptions = {
  name: "Sanshain UI tests",
  outputDir: "build/ui-coverage",
  reports: ["lcovonly"],
  // The service's own page scripts; the vendored libraries are not ours.
  entryFilter: (entry) =>
    /\/js\/[\w/.-]+\.js(\?|$)/.test(entry.url) && !entry.url.includes("/vendor/"),
  // Served under /js/, kept in the repository under static/js/.
  sourcePath: (filePath) => {
    const at = filePath.indexOf("js/");
    return at >= 0 ? `static/${filePath.slice(at)}` : filePath;
  },
};
