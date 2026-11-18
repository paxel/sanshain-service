// The global of `static/vendor/diff2html.min.js`, with only the members `static/js/audit.js`
// calls. This file has no import or export, which keeps its declarations global.

/** The options `static/js/audit.js` passes to `Diff2Html.html`. */
interface Diff2HtmlOptions {
  drawFileList?: boolean;
  matching?: "lines" | "words" | "none";
  outputFormat?: "line-by-line" | "side-by-side";
  renderNothingWhenEmpty?: boolean;
}

declare namespace Diff2Html {
  /** Renders a unified diff as HTML. */
  function html(diffInput: string, configuration?: Diff2HtmlOptions): string;
}
