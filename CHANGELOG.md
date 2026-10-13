# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.4.0] - 2026-10-13

### Added
- `PUT /catalog/{systemname}`: a build can push its Backstage `catalog-info.yaml`, and Sanshain records the **Peripheral Resources** (databases, buckets, queues…) the system's Component lists under `dependsOn`. Only `resource:` refs are read, and each push replaces the system's previous set. A trunk build updates the main graph as well as the dev graph. A file without a Component named like the system is rejected (`400`), so a name mismatch fails the build. Clients point to the file with `catalogInfo` in `sanshain.yaml`.
- `/report` carries `peripheral_dependencies` (system → resource edges) for its scope, and the new `GET /admin/periphery?scope=` answers the same edges for `dev`, `main[@instant]` or `<branch>[@instant]`. A past instant shows the periphery as it was then.
- Sanshain-branches copy the trunk periphery when they are cut (also retroactively, as of the cut's instant). The graph diff reports `periphery_added`/`periphery_removed`, and periphery changes show up as timeline instants. Deleting a Producer or Consumer closes the periphery of a system that has no role left; branch copies are kept.
- The network graph draws Peripheral Resources as their own node kind (slate, dashed outline, labelled `name` or `namespace/name`), with dashed edges from the systems that depend on them. Systems sharing a resource point at one node. A new **Periphery** switch next to the protocol filters hides them, and a legend entry highlights them. The main, branch and timeline views show the periphery of that scope and instant, and the compare panel lists added and removed periphery edges.

### Security
- Updated `quinn-proto` to 0.11.19 (RUSTSEC-2026-0339, RUSTSEC-2026-0341, RUSTSEC-2026-0342).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
