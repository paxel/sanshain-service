// Writes build/ui-coverage/lcov.info from every test's coverage after the run.
import MCR from "monocart-coverage-reports";
import { uiCoverageEnabled, uiCoverageOptions } from "./coverage.js";

export default async function teardown() {
  if (uiCoverageEnabled) await MCR(uiCoverageOptions).generate();
}
