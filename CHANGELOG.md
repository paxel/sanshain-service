# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.13.0]

### Added
- The graph's timeline shows the last 3 months by default; its start and end are UTC days you can change, and the slider spaces the changes by time.
- Graph links carry the view, the sanshain-branch, the timeline range and instant, the display switches, the direction and the graph mode, so a link shows the same graph to whoever opens it.
- `GET /admin/trunk/timeline` and `GET /admin/branches/{name}/timeline` accept optional `from` and `to`.
- A Domains switch on the graph writes the domain onto each node instead of grouping the services in frames, so the layout can arrange them freely.
- Dense, a third density step after Compact, packs the graph's nodes and domain frames tightly for small monitors.
- Single-key shortcuts on the graph page for its switches, the legend, fitting the view and stepping the timeline; `?` lists them.
- A fullscreen graph (`f`) for small monitors and screen sharing: the graph fills the window or the screen, with its controls in an icon column, the timeline as a bar and the legend as a panel.
- Experimental: `z` on the graph page collapses the graph onto its domains, with pin counts and warning colours between them; the focus field takes `domain:<name>`.
- Clear system information on a Producer's or Consumer's page (`DELETE /admin/systems/{name}/information`, an admin or the Producer's maintainer): removes the catalog context and the system annotations and closes the current periphery; the timeline and sanshain-branch copies keep theirs, and the next build sets everything again.
- Annotations: a build can send project-declared entries from `sanshain.yaml` (`annotations` on provide, require, require-bundle and catalog push), stored with the provided version (`scope: version`) or once per system (`scope: system`), and answered with the version listings, the Producer and Consumer data, the graph report, MCP `describe_service` and the `version.published` webhook. The version row, the graph's node panel and the Producer and Consumer pages show them, a value starting with `http(s)://` as a link labelled by the name.
- MCP `review_spec` checks a spec before it is provided: the provide's validation and version rules against the stored line as a dry run (GA by default, no releaser role needed, nothing stored), and the findings that matter in Sanshain: absolute server URLs, AsyncAPI messages without a name, endpoints removed without a deprecation in the previous GA, endpoints without a summary, a proto without its version marker.
- MCP `setup_guide` answers, for a system's role, build tool and CI, the client with its install line and build-file snippet, the commands, the build's token and where it goes, the variables and a `sanshain.yaml` skeleton, as structured facts.
- MCP `draft_sanshain_yaml` writes a system's `sanshain.yaml` (provides, requires, catalog info, `autoGa`, annotations) from the names and pins an assistant looked up, and answers the instance address and the environment the build needs; it changes nothing.
- Products (ADR-0011): named envelopes of systems on a new Products page, kept by Product Maintainers (users or groups, also directory groups) who pick known systems or paste the Helm umbrella `Chart.yaml` (each dependency's `alias`, else `name`; unknown names skipped and reported, members added by hand kept), each member shown with its roles or as "not in Sanshain", and a `product:<name>` graph focus; a Product grants no power over its members (routes under `/admin/products`, all audited).
- The site documents each of the five clients on a page of its own (Maven, Rust, Go, JavaScript with the GitHub Action, Conan): install, every option with the client version it arrived in, the precedence, caching, strict mode and best effort, the failure modes and CI recipes; the client READMEs now point there.

### Changed
- Each domain frame in the graph has a color of its own, with a solid border and a larger, darker name.
- The Admin link in the navigation shows only to those who may open the admin dashboard, on every page.

### Fixed
- Panning the graph no longer slows down the page after many redraws.
- A trunk provide of a new version now shows as a marker on the main graph's timeline, also without a trunk require.
- An AsyncAPI message given as a local `$ref` (`#/components/messages/…`, also inside `oneOf`) now registers its message contract on a GA provide and is harvested as a subscription; before, only messages written inline counted.
- The GitHub Actions examples of the site name the JavaScript client's Action as `paxel/sanshain-js@summer`; the `@main` they named does not exist, so the step failed to resolve.
- The Developer track's start page counts its nine chapters; it said seven.

### Security
- The first start no longer prints the initial admin's password: one set in `INITIAL_ADMIN_PASSWORD` is never shown, and a generated one is written to `initial-admin-password` next to the SQLite database (`/data` in the image and with PostgreSQL), readable by the service user only, and deleted after the first password change. A file that cannot be written stops the start.
- The image no longer installs Alpine's `sqlite-libs`: the binary carries its own SQLite, so the unused library only added scanner findings.
- Updated `rmcp` to 3.9.0 (RUSTSEC-2026-0462, unbounded response bodies in its Streamable HTTP clients; Sanshain builds only the MCP server, so the affected client transports were never compiled in).
- `cargo audit` ignores RUSTSEC-2026-0459 (CVE-2026-119375 in the SQLite that `libsqlite3-sys` 0.37 bundles: uninitialized memory returned when a database is opened together with a crafted `-wal` file): the fix needs `libsqlite3-sys` 0.39, which no released `sqlx` accepts yet, and the attack needs Sanshain to open database files someone else wrote, while it opens only its own; the ignore goes with the first `sqlx` release that takes the fix.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
