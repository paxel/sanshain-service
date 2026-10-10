### Context

![Context view](<context.svg>)

Sanshain sits between the build pipelines of many microservices: Build Tooling publishes specs as Producer and fetches pinned endpoint snippets as Consumer, while people use the web dashboard to inspect versions and the dependency graph. Human login can be delegated to LDAP/AD or an OIDC provider; machine clients always use API tokens. State lives in SQLite or PostgreSQL; traces and metrics go to an optional observability stack.

#### Communication partners

| Partner                 | Description                                                                                                                                                                      | Input                                                              | Output                                                             |
|-------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------|--------------------------------------------------------------------|
| Build Tooling           | Maven plugin, cargo-sanshain, Go CLI, JS client/GitHub Action and Conan plugin; publish specs for a Producer and fetch pinned endpoint snippets for a Consumer during CI builds. | provide / require / report (REST, API token) (Sanshain Service)    | –                                                                  |
| LDAP / Active Directory | Optional corporate directory that authenticates human users and supplies group memberships.                                                                                      | –                                                                  | bind + group lookup (LDAP/LDAPS) (Sanshain Service)                |
| Observability Stack     | Optional OpenTelemetry collector receiving traces and Prometheus scraping request metrics.                                                                                       | OTLP traces (gRPC) / Prometheus scrape /metrics (Sanshain Service) | OTLP traces (gRPC) / Prometheus scrape /metrics (Sanshain Service) |
| OIDC Provider           | Optional SSO identity provider for human login.                                                                                                                                  | –                                                                  | authorization code flow (HTTPS) (Sanshain Service)                 |
| User                    | Developer, maintainer or administrator who browses producers, versions, diffs and the dependency graph, and manages users, roles and settings.                                   | web dashboard, graph, admin (HTTPS) (Sanshain Service)             | –                                                                  |

#### Building blocks

| Name             | Responsibility                                                                                                                                                                                     |
|------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Database Server  | Stores specs, version lines, endpoints, pins, branches, users, roles and audit log. Embedded SQLite by default, PostgreSQL for larger deployments.                                                 |
| Sanshain Service | Central registry for OpenAPI, AsyncAPI and gRPC specs: Producers publish versioned specs, Consumers download only the endpoints they pin, and the resulting dependency graph is tracked and shown. |

