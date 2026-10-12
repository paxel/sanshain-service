# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.4.0] - Unreleased

### Added
- `PUT /catalog/{systemname}`: a build can push its Backstage `catalog-info.yaml`, and Sanshain records the **Peripheral Resources** (databases, buckets, queues…) the system's Component lists under `dependsOn`. Only `resource:` refs are read, and each push replaces the system's previous set. A trunk build updates the main graph as well as the dev graph. A file without a Component named like the system is rejected (`400`), so a name mismatch fails the build.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
