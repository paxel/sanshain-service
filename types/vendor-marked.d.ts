// marked 18.2.0 (`static/vendor/marked-18.2.0.umd.js`): only the members the page scripts
// call. This file has no import or export, which keeps the declaration global.

declare const marked: {
  /** Markdown to HTML. Without the `async` option, which no script sets, the result is a string. */
  parse(text: string): string;
};
