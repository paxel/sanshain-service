// The dependency graph page (`static/graph.html`): the state its scripts keep on `window`, and
// the shapes that `static/js/graph.js`, `static/js/graph-domains.js` and
// `static/js/pages/graph-1.js` share. A field is optional or nullable where the scripts guard
// it; the authority for each JSON shape is the Rust type named beside it. The top-level
// functions of the scripts need no declaration: they are globals of the one program.
// This file has no import or export, which keeps its declarations global.

/** `TrunkPinInfo` in `src/domain/models.rs`: one pin of the main graph or of a branch. */
interface GraphPin {
  client: string;
  service: string;
  api_type: string;
  version: string;
  path: string;
  method: string;
  valid_from?: string;
  last_required_at: string;
  /** Left out by the service unless true. */
  dangling?: boolean;
}

/**
 * One edge as the renderers draw it. From the service it is `DependencyInfo` in
 * `src/domain/models.rs`; the scripts build more of them from pins, harvested subscriptions,
 * the periphery and the topic explode, which is where the optional fields come from.
 */
interface GraphEdge {
  api_type: string;
  client: string;
  service: string;
  /** Null on an edge without a version: a harvested subscription or a resource edge. */
  version: string | number | null;
  stability: string | null;
  path: string;
  method: string;
  deprecated: boolean;
  last_required_at?: string;
  dangling?: boolean;
  harvested?: boolean;
  unfulfilled?: boolean;
  peripheral?: boolean;
  missing_component?: boolean;
  topic_edge?: boolean;
  topic_unowned?: boolean;
  declared_channel?: boolean;
  /** The key of the edge a topic edge was made from, for the highlight flags. */
  flag_key?: string | null;
  /** The node whose requires badge a topic edge stands for. */
  badge_requires?: string | null;
}

/** `MissingEndpointInfo` in `src/domain/models.rs`, as far as the graph reads it. */
interface GraphMissingEndpoint {
  client: string;
  service: string;
}

/** `PeripheralDependency` in `src/domain/models.rs`. */
interface GraphPeripheralDependency {
  system: string;
  resource: string;
}

/** `HarvestedSubscription` in `src/domain/models.rs`. */
interface GraphHarvestedSubscription {
  client: string;
  channel: string;
  message_name?: string;
  /** Null while nobody provides the channel. */
  owner: string | null;
  trunk?: boolean;
}

/** `ScopeChannel` in `src/domain/models.rs`. */
interface GraphScopeChannel {
  channel: string;
  method: string;
}

/** `ScopeVersionLine` in `src/domain/models.rs`. */
interface GraphScopeVersionLine {
  api_type: string;
  version: string;
  /** Null for a deleted version. */
  stability: string | null;
  last_provided_at?: string | null;
  /** Left out by the service when empty. */
  channels?: GraphScopeChannel[];
}

/** `ScopeProducer` in `src/domain/models.rs`. */
interface GraphScopeProducer {
  name: string;
  versions?: GraphScopeVersionLine[];
}

/** `GraphScopeData` in `src/domain/models.rs`, the answer of `GET /admin/graph/scope`. */
interface GraphScopeData {
  producers?: GraphScopeProducer[];
  harvested_subscriptions?: GraphHarvestedSubscription[];
  systems?: Record<string, SystemContext>;
  system_annotations?: Record<string, Annotation[]>;
}

/**
 * `DependencyReport` in `src/domain/models.rs`, the answer of `GET /report`, as far as the
 * graph reads it. `services_detailed` is added by the page from `GET /admin/producers`, and
 * `unrequired` by `getFilteredReport`.
 */
interface GraphReport {
  dependency_graph: GraphEdge[];
  missing_endpoints?: GraphMissingEndpoint[] | undefined;
  trunk_graph?: GraphPin[];
  peripheral_dependencies?: GraphPeripheralDependency[];
  systems?: Record<string, SystemContext>;
  trunk_stale_before?: string | null;
  services_detailed?: ProducerSummary[];
  unrequired?: GraphScopeProducer[];
}

/** A report of which every part may be missing: what the helpers that guard each part take. */
type GraphReportLike = Partial<GraphReport>;

/** What `getFilteredReport` returns: the report narrowed to the view and the filters. */
interface GraphFilteredReport extends GraphReportLike {
  dependency_graph: GraphEdge[];
  unrequired: GraphScopeProducer[];
}

/** What is known about the pins of one drawn edge (`graphEdgeFlags`). */
interface GraphEdgeFlags {
  outdated: boolean;
  breakingOutdated: boolean;
  snapshot: boolean;
  conflict: boolean;
  stale: boolean;
  dangling?: boolean;
}

/** The provides and requires badges of one node (`graphNodeBadges`). */
interface GraphNodeBadges {
  provides: string[];
  requires: string[];
}

/** The protocol filters of the toolbar; a name the toolbar does not know reads undefined. */
interface GraphProtocolFilters {
  openapi: boolean;
  asyncapi: boolean;
  proto: boolean;
  periphery: boolean;
  unrequired: boolean;
  [protocol: string]: boolean | undefined;
}

/** The highlight toggles. */
interface GraphHighlightFilters {
  outdated: boolean;
  breakingOutdated: boolean;
  snapshot: boolean;
}

/** The view switches of the custom renderer. */
interface GraphViewSwitches {
  rainbow: boolean;
  topics: boolean;
}

/** A range of UTC days, each written YYYY-MM-DD. */
interface GraphTimelineRange {
  from: string;
  to: string;
}

/** The two elements of one drawn edge. */
interface GraphEdgeElements {
  path: SVGPathElement;
  hitArea: SVGPathElement;
}

interface Window {
  /** "all" or "circular". */
  graphRedrawMode: string;
  /** The focus filter. */
  graphFocusTags: string[];
  graphProtocolFilters: GraphProtocolFilters;
  graphViewSwitches: GraphViewSwitches;
  /** "normal", "compact" or "dense". */
  graphDensity: string;
  GRAPH_DENSITIES: string[];
  /** "group" or "node". */
  graphDomainsMode: string;
  /** "services" or "domains". */
  graphLevel: string;
  GRAPH_HOVER_DELAY_MS: number;
  GRAPH_DOMAIN_PALETTE: { border: string; text: string }[];
  graphHighlightFilters: GraphHighlightFilters;
  /** "dev", "main" or "branch". */
  graphStreamView: string;
  graphBranchName: string | null;
  graphBranchPins: GraphPin[] | null;
  /** Every sanshain-branch, once the page has loaded them. */
  graphBranchNames?: string[];
  /** The instant on screen; null is now. */
  graphTimelineAt: string | null;
  /** The instant asked for last, applied or still loading. */
  graphTimelineTarget?: string | null;
  graphTimelinePins: GraphPin[] | null;
  graphTimelineDates: string[];
  graphTimelineRange: GraphTimelineRange;
  graphTimelineRangeChosen: boolean;
  graphTimelineNowMs: number;
  graphUserPermissions: string[];
  graphScopePeriphery: GraphPeripheralDependency[] | null;
  graphScopeData: GraphScopeData | null;
  /** Counts the timeline requests, so only the newest applies its result. */
  _graphTimelineToken?: number;
  /** Counts the view switches made by the user. */
  _graphUserViewSwitches?: number;
  /** The drawn nodes by name, for the legend and the highlights. */
  _graphNodeElements?: Map<string, SVGGElement>;
  /** The drawn edges by "from-->to". */
  _graphEdgeElements?: Map<string, GraphEdgeElements>;
}
