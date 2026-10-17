# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.5.0]

### Added
- API tokens have a scope: `full` (as before) or `read_only`, chosen with a "Read-only (for AI / MCP clients)" checkbox when the token is created and immutable afterwards. A read-only token may read reports, versions and the update streams but is refused with `403` on every route that changes the catalog (provide, retire, require, catalog push).
- The account page shows each token's scope and when it last authenticated a request.
- An MCP endpoint at `/mcp` (Streamable HTTP, stateless) for AI clients, authenticated with an API token of either scope; `MCP_ENABLED`, `MCP_CALLS_PER_MINUTE` and `MCP_MAX_RESULT_BYTES` configure it, and `sanshain_mcp_tool_calls_total` / `sanshain_mcp_result_bytes_total` count its tool calls.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
