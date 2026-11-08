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

### Changed
- Each domain frame in the graph has a color of its own, with a solid border and a larger, darker name.
- The Admin link in the navigation shows only to those who may open the admin dashboard, on every page.

### Fixed
- Panning the graph no longer slows down the page after many redraws.
- A trunk provide of a new version now shows as a marker on the main graph's timeline, also without a trunk require.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
