# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.7.0]

### Changed
- `maintenance.yaml` declares the permission and scope of every administrative operation (`x-permission`, `x-scope`, `x-api-tokens`) and `api.yaml` the token rule of every operation (`x-auth`, with a declared bearer security scheme); a build-time test keeps them equal to the router's guards, and the documentation site reads them instead of the prose.
- `api.yaml` documents the routes an API token could already reach but the contract did not list: `/report`, `/report/markdown`, `/report/isolation`, `/endpoint-versions`, `/api/sse/updates`, `/api/ws/updates` and the session-only `/api/audit/timeline`, with their schemas; the REST reference shows them.
- `api.yaml` carries a recorded example for every request body and answer of the Producer and Consumer API (the 409 with `proposed_version`, the read-only refusal, a require snippet, the report, the history, a token), taken from a live 2.6.1 instance; the REST reference shows them instead of placeholders.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
