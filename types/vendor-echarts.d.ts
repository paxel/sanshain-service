// Apache ECharts 6.1.0 (`static/vendor/echarts-6.1.0.min.js`), the charts of the statistics
// page: only the members the page scripts call. This file has no import or export, which keeps
// the declaration global.

declare namespace echarts {
  interface ECharts {
    setOption(option: Record<string, unknown>, notMerge?: boolean): void;
    /** The chart as an SVG document; only with the `svg` renderer. */
    renderToSVGString(): string;
    resize(): void;
    dispose(): void;
    getWidth(): number;
    getHeight(): number;
    on(event: string, handler: (params: { dataIndex: number }) => void): void;
  }
  function init(
    element: HTMLElement,
    theme?: string | null,
    options?: { renderer?: "svg" | "canvas"; width?: number; height?: number },
  ): ECharts;
}
