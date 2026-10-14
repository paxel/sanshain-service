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

### Changed
- Provides normalize the uploaded document before hashing, checking and storing it: CRLF and lone CR line endings become LF and a leading UTF-8 BOM is removed. `content_hash` and the stored document are those of the normalized text, and `/validate` checks the same normalized text.
- The first start of 2.4.0 rewrites spec text stored by earlier releases the same way, once (ADR-0008): stored documents, gRPC snippets and channel-message contract payloads lose their carriage returns and BOM, and `content_hash` is recomputed. GA versions keep their stability, attribution and timestamps.
- The spec and endpoint viewer's blame groups consecutive lines last changed in the same version into one block: user, version and date appear once on the block's first line, a gutter bar spans the block and neighbouring blocks alternate background. Blame is computed by the service (`GET /admin/producers/{name}/blame`) with the same diff as the diff views.

### Fixed
- The spec viewer's diff of large documents shows the real changes instead of marking most of the document as removed and re-added.
- Re-providing a GA spec from a Windows checkout (CRLF line endings) is the idempotent no-op instead of a `409` immutability conflict, and a document starting with a UTF-8 BOM is accepted instead of rejected as unparsable YAML.
- gRPC snippets split from a CRLF `.proto` no longer carry stray carriage returns.

### Security
- Updated `quinn-proto` to 0.11.19 (RUSTSEC-2026-0339, RUSTSEC-2026-0341, RUSTSEC-2026-0342).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
