# MCP: Sanshain for AI clients

Sanshain exposes its catalog to AI assistants over the [Model Context Protocol](https://modelcontextprotocol.io)
(MCP). An assistant with the endpoint configured can answer questions like "who provides the orders
API and who depends on it?", "where is the endpoint that creates an invoice?", "what changed in
`payments` between 2.3.0 and 2.4.0?" or "show me the spec of `inventory`", straight from the catalog —
read-only, and only the catalog: never users, roles, settings, authentication configuration, audit or
observability data.

### TL;DR
- **Endpoint**: `POST /mcp` (Streamable HTTP, stateless). Also answers `GET`/`DELETE` as the transport requires.
- **Auth**: `Authorization: Bearer <API token>`. Session tokens are refused. Mint a **read-only** token for AI clients.
- **Tools**: `list_services`, `describe_service`, `find_endpoint`, `get_spec`, `get_graph`, `diff_versions`, `blame`.
- **Limits**: 60 tool calls per minute per token, 64 KiB per answer (larger answers come back as a summary with a hint).
- **Off switch**: `MCP_ENABLED=false` leaves `/mcp` unmounted.

---

## Token setup

The MCP endpoint authenticates with Sanshain API tokens — the same tokens CI uses — and nothing else.
Create one on the account page (`/account.html`) and tick **Read-only (for AI / MCP clients)**: a
`read_only` token can query everything the tools answer but is refused on every route that changes
the catalog (provide, retire, require, catalog push), so a misdirected assistant cannot publish or pin
anything. See [API Usage](api-usage.md#api-tokens) for the scope rules.

A `full` token works too; prefer read-only unless the same token also drives a build.

## The endpoint

`/mcp` speaks MCP's Streamable HTTP transport in stateless mode: every request carries the token,
no session is kept, and a Sanshain restart loses nothing the client could notice. JSON-RPC requests
are `POST`ed with `Accept: application/json, text/event-stream`; plain request/response tools are
answered as JSON.

| Condition                                   | Answer                                                                                                                           |
|---------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------|
| No `Authorization` header                   | `401` with `WWW-Authenticate: Bearer`                                                                                            |
| Session token, unknown or expired API token | `401`                                                                                                                            |
| `MCP_ENABLED=false`                         | `404` — the route is not mounted                                                                                                 |
| Over the rate limit                         | A tool result with `isError: true` and `rate limited - retry in N s` (never an HTTP 429, which would break the client's session) |
| Answer above `MCP_MAX_RESULT_BYTES`         | That tool's summary and a hint on how to narrow the question                                                                     |
| Unknown service, version or endpoint        | A tool result with `isError: true` and a message naming it                                                                       |

Dev mode (`SANSHAIN_DEV_MODE` with `ALLOW_INSECURE_DEV_MODE`) substitutes the dev user when no
header is sent, exactly as the API does. Local only.

## Client configuration

**Claude Code**

```bash
claude mcp add --transport http sanshain https://<host>/mcp --header "Authorization: Bearer <token>"
```

**Any stdio-only client** via the `mcp-remote` bridge:

```json
{
  "mcpServers": {
    "sanshain": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "https://<host>/mcp", "--header", "Authorization: Bearer <token>"]
    }
  }
}
```

**Cursor** (`.cursor/mcp.json`) and **VS Code** (`.vscode/mcp.json`):

```json
{
  "mcpServers": {
    "sanshain": {
      "url": "https://<host>/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

VS Code spells the top-level key `servers` and the transport `"type": "http"`:

```json
{
  "servers": {
    "sanshain": {
      "type": "http",
      "url": "https://<host>/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

Keep the token out of files you commit — most clients read it from an environment variable
(`${env:SANSHAIN_TOKEN}` in VS Code, `${SANSHAIN_TOKEN}` in Cursor).

## Tools

Every tool answers compact JSON stamped with the `scope` (and `at`) it was computed for. Scopes are
the same as the reports': `dev` (default) is everything ever provided or required, `main` is what
trunk builds currently pin, and a sanshain-branch name is a release cut; `main` and branches take an
`at` instant on `get_graph`. Unknown names are refused with a message, never with a transport error.

| Tool               | Parameters                                                                    | Answers, and an example question                                                                                                                                                                                                                     |
|--------------------|-------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `list_services`    | `scope?`                                                                      | Producers (domain, icon, API types, version line per type, endpoint and consumer counts, `unrequired`) and Consumers (what they require). *"Which services are in production?"*                                                                      |
| `describe_service` | `name`, `scope?`                                                              | One service: version lines (with `last_provided_at`, `superseded`), endpoints of the latest version per line, its consumers and what they pin, what it requires, its periphery and who shares it. *"Who depends on `orders`, and on which version?"* |
| `find_endpoint`    | `query`, `method?`, `api_type?`, `all_versions?`                              | Endpoints across all Producers whose path, channel or method name contains `query` (case-insensitive, `*` wildcard), latest versions by default, with the consumer count. *"Where is the endpoint that cancels an order?"*                           |
| `get_spec`         | `service`, `api_type`, `version?`, `path?`, `method?`, `offset?`, `limit?`    | The stored document (latest version when `version` is omitted) or one endpoint's snippet; paged by line with `offset`/`limit` and `next_offset`. *"Show me the request body of `POST /orders`."*                                                     |
| `get_graph`        | `scope?`, `at?`, `service?`, `api_type?`, `include_periphery?`                | Nodes (`producer`, `consumer`, `both`, `missing`, `resource`) and edges (api_type, method, path, version, stability, `outdated`, `superseded`). *"What did the release graph look like on the 1st?"*                                                 |
| `diff_versions`    | `service`, `api_type`, `from`, `to`, `ignore_whitespace?`, `path?`, `method?` | The unified diff (the UI's engine) and the added/removed line counts. *"What changed between 2.3.0 and 2.4.0 of `payments`?"*                                                                                                                        |
| `blame`            | `service`, `api_type`, `version`, `path?`, `method?`                          | Blocks of lines with the version that last changed them, who provided it and when. *"Since when has this field been required?"*                                                                                                                      |

`api_type` is `openapi`, `asyncapi` or `proto`; versions are `MAJOR.MINOR.PATCH`; `path` and `method`
always go together. Every schema refuses unknown parameters.

### Over-budget answers

An answer above `MCP_MAX_RESULT_BYTES` is never truncated silently. Instead:

- `get_spec` answers `{summary: {lines, bytes, endpoints: [...]}, hint: "use offset/limit (line numbers) or path+method"}`;
- `get_graph` answers the node counts per kind and the edge count, with a hint to narrow with `service` or `api_type`;
- `diff_versions` answers the added/removed counts with a hint to narrow with `path` + `method`;
- `blame` answers the blocks without their lines (`line_count` instead);
- `list_services`, `describe_service` and `find_endpoint` answer counts and a hint.

## Limits and settings

| Variable               | Default | Meaning                                                                                        |
|------------------------|---------|------------------------------------------------------------------------------------------------|
| `MCP_ENABLED`          | `true`  | Mount `/mcp`. `false` leaves it unmounted (404).                                               |
| `MCP_CALLS_PER_MINUTE` | `60`    | Tool calls per minute per API token (token bucket: a burst of 60, refilled continuously). > 0. |
| `MCP_MAX_RESULT_BYTES` | `65536` | The size budget of one tool answer; above it the tool's summary is answered instead. > 0.      |

`initialize` and `tools/list` do not count against the rate limit; only tool calls do. The HTTP body
limit (`MAX_SPEC_BODY_BYTES`) applies to requests as everywhere else.

Two Prometheus counters on `/metrics` describe the traffic:
`sanshain_mcp_tool_calls_total{tool, outcome}` with `outcome` one of `ok`, `error`, `rate_limited`,
`over_budget`, and `sanshain_mcp_result_bytes_total{tool}`.

## Security notes

- Only API tokens are accepted, so there is no ambient credential a browser could ride; the endpoint
  is outside CSRF protection for that reason.
- `Host`-header validation is disabled on the transport: Sanshain is reached under whatever name the
  deployment gave it, and the bearer token — not the `Host` header — keeps a DNS-rebinding attacker out.
- A token's `last_used_at` on the account page shows when an assistant last used it.
