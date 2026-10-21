# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.9.0]

### Added
- `sanshain_background_job_failures_total{job}` counts every failed run of a periodic job (snapshot, dependency, trunk, credential and webhook cleanup, webhook delivery), so monitoring can alert on a job that fails every run instead of an unread WARN line.
- A **Ticket Links** setting on the admin page: a URL template with `{id}` (`https://jira.acme.corp/browse/{id}`) turns the ticket ids of a version's origin into links on the version row, in the diff header and the blame; unset shows them as plain text.
- A provide may say why: `build_url`, `change_url` (the pull or merge request), `tickets` (ids) and `note` (one line) are stored with the version as its origin. The latest provide that carries any of them wins; a no-op re-provide of identical bytes only fills an empty one. Shown on the version row, in the diff header and every blame block, answered by the MCP (`describe_service`, `diff_versions`, `blame`) and carried by the `version.published` webhook.
- A **Topics** graph switch beside Compact and Rainbow: every AsyncAPI channel becomes a node between its publishers and subscribers in place of the direct messaging arrows, a channel nobody publishes stands as a dashed topic, and the topic's panel lists who publishes and who subscribes. Remembered in the browser like the other switches.
- A `component:` ref in the Component's `dependsOn` that names no Producer and no Consumer is drawn as a dashed missing component with an edge from the depending system (legend entry, panel naming who declared it); a ref with a Sanshain pendant stays ignored, so the moment that system provides or requires, the `require` edges take over (ADR-0010).
- The catalog push keeps the Component's `metadata.tags`: chips with the catalog context on the node panel and the Producer and Consumer pages, `tags` in the MCP `describe_service` answer, and `tag:<name>` in the graph's focus box narrows the drawing to the tagged systems and their neighbours.
- The catalog push reads the Component's `spec.system`: its name fills the Producer's domain, the cluster the graph draws, when no domain is set; a domain an administrator typed stays (ADR-0010).
- Endpoints carry the one line their Producer wrote about them: an OpenAPI operation's `summary` (or the first sentence of its `description`), an AsyncAPI operation, message or channel summary, or the comment above a proto `rpc`. The Producer page shows it under each endpoint, and the MCP `find_endpoint` hits and `describe_service` endpoints carry it as `summary`, so an assistant answers "what is this for" without fetching the spec. Proto snippets now keep the comment block above their `rpc`.

### Security
- `rustls-webpki` 0.103.16 and `webpki-roots` 1.0.10, closing RUSTSEC-2026-0382 (distrusted intermediates accepted) and RUSTSEC-2026-0381 (distrusted emSign intermediates in the bundled roots).

### Fixed
- An expired session or API token stopped working only at the end of its expiry day (UTC), not at its expiry time: the check compared the stored `…T…` timestamp with a `… …` one as text. Sessions on SQLite and API tokens on both databases were affected; both now compare instants.
- On PostgreSQL the hourly credential cleanup failed every run (`operator does not exist: timestamp without time zone < text`) and removed no expired session or API token; it now cleans both, and a failure on one no longer skips the other.

### Changed
- Every client and server error in the log names the request and the caller: `Client error: Forbidden (POST /admin/settings/cache, user dev-1)` instead of a bare `Client error: Forbidden`. The query string and any credential stay out of the line.
- Graph panels open after the mouse has rested on a node or edge for two seconds instead of at once, so a cursor crossing the graph no longer pops panels along its way; once a panel is open, moving onto a neighbour switches it at once, and clicking an `(unrequired)` chip opens its panel immediately as before.
- A snapshot can only be changed by the user who last provided it. Different content from anyone else is refused `409` (`snapshot_owned`) naming the provider and proposing the next free minor, so two developers who picked the same number no longer break each other's Consumer builds; identical bytes from anyone (CI rebuilding an already provided commit) stay a no-op, and an administrator deletes the snapshot to free the number.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
