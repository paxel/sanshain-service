// The JSON shapes and the script-to-script contracts the catalogue pages share: the Producer and
// Consumer lists, the spec viewer with its structure view, the reports and the observability
// page. A field is optional or nullable here where the scripts guard it; the authority for each
// shape is the Rust type named beside it. A shape only one page reads is a typedef in that
// page's script. These are global declarations: this file has no import or export.

/** The endpoint filter of a list, as the URL holds it (`static/js/endpoint-filter.js`). */
interface EndpointFilter {
  /** The search words. */
  q: string;
  /** At most one of `used` and `unused`, plus `deprecated`. */
  show: Set<string>;
}

/** A chip of the endpoint filter; `all` comes with `used` and `unused`. */
type EndpointFilterChip = "all" | "used" | "unused" | "deprecated";

/** What the endpoint filter reads of an endpoint; `used` is set on the Producer view only. */
interface FilterableEndpoint {
  method: string;
  path: string;
  summary?: string | null | undefined;
  deprecated?: boolean | undefined;
  used?: boolean | undefined;
}

/** What `renderEndpointFilter` hands back. */
interface EndpointFilterControl {
  /** Empties the filter and redraws. */
  clear(): void;
}

/** `ParticipantContext` in `src/domain/models.rs`: what `GET /admin/producers/{name}` and `GET /admin/consumers/{name}` answer. */
interface ParticipantContext {
  context?: SystemContext | null;
  annotations?: Annotation[] | null;
}

/** `BranchInfo` in `src/domain/models.rs`, as far as the catalogue pages read `GET /admin/branches`. */
interface CatalogBranch {
  name: string;
}

/** `Node` in `src/structure.rs`: one node of the structure view. The server leaves out what is empty or false. */
interface StructureNode {
  key: string;
  kind: string;
  name: string;
  method?: string;
  path?: string;
  type?: string;
  summary?: string;
  description?: string;
  required?: boolean;
  deprecated?: boolean;
  nullable?: boolean;
  read_only?: boolean;
  write_only?: boolean;
  default?: unknown;
  example?: unknown;
  example_generated?: boolean;
  enum?: unknown[];
  constraints?: string[];
  origin?: string;
  discriminator?: string;
  /** The version that last changed this node. */
  changed_in?: string;
  /** The version that last changed something below this node. */
  inside?: string;
  children?: StructureNode[];
  /** Set on an endpoint whose children load when it is opened. */
  children_omitted?: boolean;
}

/** `StructureView` in `src/application/structure_service.rs`: `GET /admin/producers/{name}/structure`. */
interface StructureView {
  service?: string;
  api_type?: string;
  version: string;
  endpoints?: StructureNode[];
}

/** What the spec viewer hands to `renderStructureView` (`static/js/structure.js`). */
interface StructureViewOptions {
  /** Whether the sections of an endpoint start open. */
  unfold: boolean;
  /** Answers one endpoint's view, for endpoints that arrive folded. */
  loadEndpoint: (path: string, method: string) => Promise<StructureView>;
  /** Copies an example to the clipboard. */
  copy: (text: string) => void;
}

interface Window {
  /** Opens a report at the chosen graph scope; set by `static/js/pages/reports.js` for its buttons. */
  openScopedReport: (src: string, title: string) => void;
}
