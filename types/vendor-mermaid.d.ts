// mermaid 11.16.1 (`static/vendor/mermaid-11.16.1.min.js`): only the members the page scripts
// call. This file has no import or export, which keeps the declaration global.

declare const mermaid: {
  initialize(config: {
    startOnLoad?: boolean;
    theme?: string;
    themeVariables?: Record<string, string>;
  }): void;
  /** Renders the diagram source found in each of the nodes. */
  run(options?: { nodes?: ArrayLike<HTMLElement> }): Promise<void>;
};
