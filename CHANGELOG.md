# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.17.0]

### Added
- A statistics page on Reports, with one tab per topic, one time range for the page (30 days, 90 days, 12 months or from–to) carried in the shareable URL, and CSV, SVG, PNG and Print on every panel; it needs the new `view_statistics` permission, held by `admin` and the new role `report_viewer` ([san-1021](https://github.com/paxel/sanshain-service/issues/134)).
- Statistics, Landscape tab: counts and member lists of Producers, Consumers, Products, domains, Backstage tags and Peripheral Resources, systems active in the range against systems on main, the API type mix, a sankey of the dependencies between domains, and the External Producers ([san-1022](https://github.com/paxel/sanshain-service/issues/135)).
- Statistics, Activity tab: provides, requires and catalog pushes over time and per owner (the system's Backstage owner, never a person), the busiest systems, and refused provides over time ([san-1023](https://github.com/paxel/sanshain-service/issues/136)).
- Statistics, Versions and releases tab: versions created over time as GA or snapshot, promotions, versions per Producer, retired API families, Outdated and Snapshot-pinned Consumers, and deprecated endpoints still required ([san-1024](https://github.com/paxel/sanshain-service/issues/137)).
- Statistics, Tags and branches tab: the sanshain-branches with their cut date and member versions, and the builds per stream (dev, trunk, tag) over time and per tag ([san-1025](https://github.com/paxel/sanshain-service/issues/138)).
- Statistics, Users and access tab: users per role, users waiting for approval, groups by source, API tokens by scope and the sign-in mode, as counts without names ([san-1026](https://github.com/paxel/sanshain-service/issues/139)).
- Statistics, Catalog tab: documentation of the data, a sheet per system with what it provides (with the Origin of the latest version), what it consumes, who consumes it, its owner, domain, Products, tags, links and Annotations; filters by owner, domain, Product and tag, Print for one sheet and Print all filtered, one sheet per page ([san-1027](https://github.com/paxel/sanshain-service/issues/140)).

### Changed
- The Reports page and its nav link need `view_statistics`; signed-in users without it no longer see them ([san-1021](https://github.com/paxel/sanshain-service/issues/134)).
- The Producer isolation report and the full dependency report are Landscape panels of the statistics page, sortable, the dependencies filterable and exported as CSV, instead of Markdown tables in a viewer; `/report`, `/report/markdown` and `/report/isolation` are unchanged ([san-1028](https://github.com/paxel/sanshain-service/issues/141)).
- The version a `409` proposes is one minor above the highest version of the same major, or the highest major plus one for a breaking change; a patch is no longer proposed ([san-1020](https://github.com/paxel/sanshain-service/issues/133)).
- Dependencies brought up to date: the graph page loads Mermaid 12.3.0 and dagre 3.1.3, and the service uuid 1.29.2, rustls-pki-types 1.16.0 and flate2 1.1.11 ([san-1019](https://github.com/paxel/sanshain-service/issues/132)).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
