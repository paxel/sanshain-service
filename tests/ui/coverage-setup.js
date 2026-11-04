// Clears the coverage of an earlier run before the UI tests start.
import MCR from "monocart-coverage-reports";
import { uiCoverageEnabled, uiCoverageOptions } from "./coverage.js";

export default function setup() {
  if (uiCoverageEnabled) MCR(uiCoverageOptions).cleanCache();
}
