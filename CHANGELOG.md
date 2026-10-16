# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.5.0]

### Added
- API tokens have a scope: `full` (as before) or `read_only`, chosen with a "Read-only (for AI / MCP clients)" checkbox when the token is created and immutable afterwards. A read-only token may read reports, versions and the update streams but is refused with `403` on every route that changes the catalog (provide, retire, require, catalog push).
- The account page shows each token's scope and when it last authenticated a request.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
