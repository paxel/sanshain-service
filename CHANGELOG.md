# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.17.0]

### Added
- A statistics page on Reports, with one tab per topic, one time range for the page (30 days, 90 days, 12 months or from–to) carried in the shareable URL, and CSV, SVG, PNG and Print on every panel; it needs the new `view_statistics` permission, held by `admin` and the new role `report_viewer` ([san-1021](https://github.com/paxel/sanshain-service/issues/134)).
- Statistics, Landscape tab: counts and member lists of Producers, Consumers, Products, domains, Backstage tags and Peripheral Resources, systems active in the range against systems on main, the API type mix, a sankey of the dependencies between domains, and the External Producers ([san-1022](https://github.com/paxel/sanshain-service/issues/135)).

### Changed
- The Reports page and its nav link need `view_statistics`; signed-in users without it no longer see them ([san-1021](https://github.com/paxel/sanshain-service/issues/134)).
- The version a `409` proposes is one minor above the highest version of the same major, or the highest major plus one for a breaking change; a patch is no longer proposed ([san-1020](https://github.com/paxel/sanshain-service/issues/133)).
- Dependencies brought up to date: the graph page loads Mermaid 12.3.0 and dagre 3.1.3, the report viewer marked 18.2.0, and the service uuid 1.29.2, rustls-pki-types 1.16.0 and flate2 1.1.11 ([san-1019](https://github.com/paxel/sanshain-service/issues/132)).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
