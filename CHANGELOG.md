# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.4.0] - Unreleased

### Added
- `PUT /catalog/{systemname}`: a build can push its Backstage `catalog-info.yaml`, and Sanshain records the **Peripheral Resources** (databases, buckets, queues…) the system's Component lists under `dependsOn`. Only `resource:` refs are read, and each push replaces the system's previous set. A trunk build updates the main graph as well as the dev graph. A file without a Component named like the system is rejected (`400`), so a name mismatch fails the build. Clients point to the file with `catalogInfo` in `sanshain.yaml`.
- `/report` carries `peripheral_dependencies` (system → resource edges) for its scope, and the new `GET /admin/periphery?scope=` answers the same edges for `dev`, `main[@instant]` or `<branch>[@instant]`. A past instant shows the periphery as it was then.
- Sanshain-branches copy the trunk periphery when they are cut (also retroactively, as of the cut's instant). The graph diff reports `periphery_added`/`periphery_removed`, and periphery changes show up as timeline instants. Deleting a Producer or Consumer closes the periphery of a system that has no role left; branch copies are kept.
- The network graph draws Peripheral Resources as their own node kind (slate, dashed outline, labelled `name` or `namespace/name`), with dashed edges from the systems that depend on them. Systems sharing a resource point at one node. A new **Periphery** switch next to the protocol filters hides them, and a legend entry highlights them. The main, branch and timeline views show the periphery of that scope and instant, and the compare panel lists added and removed periphery edges.
- Version diffs on the Producers page and in the full-spec viewer have an **Ignore whitespace** switch (off by default), and these and the endpoint viewer's diff a **Unified / Side by side** switch (side by side by default on wide screens, remembered per browser, always unified below 768px). `GET /admin/producers/{name}/diff` takes `ignore_whitespace`, `context` (`all` for the whole document), `format=json` and, with `path` and `method`, diffs one endpoint's snippet.
- `format=openapi` on `GET /require/asyncapi` and `POST /require-bundle` (query parameter) answers an AsyncAPI requirement as an OpenAPI 3.0.3 document holding only the required messages' payload schemas; `format: openapi` in `sanshain.yaml` requests it. Unknown formats, other API types, schema name clashes and non-AsyncAPI payload schema formats (e.g. Avro) are rejected with `400`.
- `GET /admin/graph/scope?scope=` answers, for `dev`, `main[@instant]` or `<branch>[@instant]`, the producers in that scope at that instant with their version lines (API type, version, stability, last provided) and the scope's harvested subscriptions. Trunk now records the version each trunk-flagged provide carries, so the main view at a past instant knows its producers; versions already marked as trunk's are recorded from their current marker onwards.
- The network graph collects the producers of the view that nobody requires and that require nothing into one **(unrequired)** box of compact name chips at the end of the flow (below a vertical graph, right of a horizontal one), in both renderers (a mermaid `subgraph`). Clicking a chip opens the node panel with the producer's version lines in that view (e.g. `openapi 1.4.0 ga · asyncapi 0.3.0 snapshot`) and when it was last provided. An **Unrequired** switch next to Periphery hides it (remembered in the URL), protocol and focus filters narrow it, and the Circular view hides it. In the main view it replaces the loose trunk-producer nodes.
- Disconnected islands — a system nobody requires and that requires nothing, with its Peripheral Resources — are packed into empty space inside the graph instead of widening it; what does not fit goes to the end of the flow, before the (unrequired) box.
- A snapshot whose version is below the latest GA of its version line is **superseded**: the Producers page marks it `superseded by X ga`, and the graph's Outdated highlight counts a pin to it as outdated and names that GA in the edge tooltip.

### Changed
- Provides normalize the uploaded document before hashing, checking and storing it: CRLF and lone CR line endings become LF and a leading UTF-8 BOM is removed. `content_hash` and the stored document are those of the normalized text, and `/validate` checks the same normalized text.
- The first start of 2.4.0 rewrites spec text stored by earlier releases the same way, once (ADR-0008): stored documents, gRPC snippets and channel-message contract payloads lose their carriage returns and BOM, and `content_hash` is recomputed. GA versions keep their stability, attribution and timestamps.
- The spec and endpoint viewer's blame groups consecutive lines last changed in the same version into one block: user, version and date appear once on the block's first line, a gutter bar spans the block and neighbouring blocks alternate background. Blame is computed by the service (`GET /admin/producers/{name}/blame`) with the same diff as the diff views.
- Graph nodes show their protocols as badges instead of the ✉/⛓️/⬡ glyphs: solid badges on the top edge for what a node provides (REST, gRPC, messaging, Peripheral Resource), outlined badges on the bottom edge for how it requires others, derived from the view's version lines and edges at the viewed instant. The mermaid renderer shows them as a second label line (`▲ REST · MSG   ▼ gRPC`), and the legend explains them.

### Removed
- The graph's virtual `BROKER` node and its "register" edges. A harvested subscription whose channel nobody provides points at a dashed missing node named `<channel> ?` instead, in both graph renderers.

### Fixed
- A graph node no longer keeps a messaging or gRPC mark (and colour role) after that version line was deleted, and the main view no longer marks a node for a line that is not trunk-provided; marks follow the view's version lines.
- The spec viewer's diff of large documents shows the real changes instead of marking most of the document as removed and re-added.
- Re-providing a GA spec from a Windows checkout (CRLF line endings) is the idempotent no-op instead of a `409` immutability conflict, and a document starting with a UTF-8 BOM is accepted instead of rejected as unparsable YAML.
- gRPC snippets split from a CRLF `.proto` no longer carry stray carriage returns.
- Graph provides badges in the main view at a past instant and in branch views now come from that scope's version lines, instead of from whatever its pins required of the producer.
- The graph's main, branch and timeline views no longer draw every harvested subscription: live main shows the trunk-flagged ones, branch views and past instants none, as the dev view always showed all.
- The graph's node panel shows what the node provides and requires instead of the producer's sticky tags, which kept naming protocols whose version lines were deleted.

### Security
- Updated `quinn-proto` to 0.11.19 (RUSTSEC-2026-0339, RUSTSEC-2026-0341, RUSTSEC-2026-0342).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
