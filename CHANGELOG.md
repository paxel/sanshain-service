# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.5.1] - 2026-07-22

### Fixed
- The endpoint version-history view (`yaml.html`) no longer shows an empty *"No version history found for this endpoint"* screen for services whose branch has no recorded history — most commonly a freshly uploaded service that has no protected `master`/`main` branch yet. Endpoint versions are only recorded on protected branches, so in 1.5.0 every endpoint of such a service displayed the empty message and no spec at all. The view now falls back to the endpoint's current stored spec (served by `GET /admin/endpoints/yaml`, which is protection-agnostic), rendered as a single current version with a note that version tracking begins on protected branches.


---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
