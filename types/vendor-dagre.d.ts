// dagre 3.1.3 (`static/vendor/dagre-3.1.3.min.js`), the layout library of the dependency
// graph: only the members the page scripts call. This file has no import or export, which
// keeps the declaration global.

declare namespace dagre {
  /** The graph's own label: the layout options going in, the laid-out size coming out. */
  interface GraphLabel {
    rankdir?: string;
    nodesep?: number;
    ranksep?: number;
    edgesep?: number;
    marginx?: number;
    marginy?: number;
    width?: number;
    height?: number;
  }

  /** What `setNode` takes. */
  interface NodeInput {
    label?: string;
    width?: number;
    height?: number;
    x?: number;
    y?: number;
    clusterLabelPos?: string;
  }

  /** A node after the layout. */
  interface Node {
    label?: string;
    width: number;
    height: number;
    x: number;
    y: number;
  }

  interface Point {
    x: number;
    y: number;
  }

  /** An edge's label; the layout adds the points. */
  interface EdgeLabel {
    points?: Point[];
  }

  interface Edge {
    v: string;
    w: string;
  }

  namespace graphlib {
    class Graph {
      constructor(options?: { compound?: boolean; directed?: boolean; multigraph?: boolean });
      setGraph(label: GraphLabel): this;
      graph(): GraphLabel;
      setDefaultEdgeLabel(label: () => EdgeLabel): this;
      setNode(name: string, label: NodeInput): this;
      node(name: string): Node;
      nodes(): string[];
      setParent(name: string, parent: string): this;
      children(name: string): string[] | undefined;
      setEdge(from: string, to: string): this;
      edge(edge: Edge): EdgeLabel;
      edge(from: string, to: string): EdgeLabel;
      edges(): Edge[];
    }
  }

  function layout(graph: graphlib.Graph): void;
}
