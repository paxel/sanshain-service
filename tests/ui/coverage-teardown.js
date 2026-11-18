// Writes coverage/ui.lcov from every test's coverage after the run.
import { copyFileSync } from "node:fs";
import MCR from "monocart-coverage-reports";
import {
  uiCoverageEnabled,
  uiCoverageLcov,
  uiCoverageOptions,
  uiCoverageReport,
} from "./coverage.js";

export default async function teardown() {
  if (!uiCoverageEnabled) return;
  await MCR(uiCoverageOptions).generate();
  copyFileSync(uiCoverageReport, uiCoverageLcov);
}
