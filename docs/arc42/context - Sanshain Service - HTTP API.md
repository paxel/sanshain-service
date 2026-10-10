### Whitebox HTTP API

![Whitebox HTTP API](<context - Sanshain Service - HTTP API.svg>)

#### Motivation

The endpoint groups follow the public REST surface documented in api.yaml: Build Tooling provides, requires and pulls reports; the Web UI uses reports, live updates, administration and login. Every group hands its work to the Application Services. Authentication, permission guards, CSRF and observability apply to every request alike, so they are drawn as the Request Guards band rather than as a box every line passes through.

#### Contained building blocks

| Name                        | Responsibility                                                                                                                                                                                                                                                                        |
|-----------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Admin Endpoints             | /admin/* (handlers/admin.rs, handlers/roles.rs): producers, versions, promotion, diffs, branches, trunk graph and timelines, consumers, users, roles, groups, maintainers, settings, cleanup, observability and audit.                                                                |
| Auth Endpoints              | /auth/login, /auth/oidc/login and callback, /auth/logout, /auth/register, /auth/me and /tokens (handlers/auth.rs): human login, sessions and API token management.                                                                                                                    |
| Live Update Endpoints       | GET /api/sse/updates and /api/ws/updates, plus /api/audit/timeline (handlers/api.rs): push provide/require events to the dashboard and graph.                                                                                                                                         |
| Provide Endpoints           | POST /provide, /provide/asyncapi, /provide/grpc (handlers/api.rs): a Producer publishes an OpenAPI, AsyncAPI or Protobuf spec version.                                                                                                                                                |
| Report & Validate Endpoints | GET /report, /report/markdown, /report/isolation and the anonymous POST /validate (handlers/api.rs): dependency reports and the free spec validator.                                                                                                                                  |
| Request Guards              | Cross-cutting request handling every endpoint passes (middleware.rs, lib.rs): API token and session authentication, per-route permission guards, CSRF validation, CORS, compression, tracing and Prometheus layers feeding Telemetry, plus health/ready/metrics and HTML page routes. |
| Require Endpoints           | GET /require, /require/asyncapi, /require/grpc, POST /require-bundle, GET /endpoint-versions and /producers/{name}/versions (handlers/api.rs): a Consumer fetches pinned endpoint snippets and discovers versions.                                                                    |

#### External interfaces

| Partner outside                                                                                         | Handled by                  | Direction | Text                                         |
|---------------------------------------------------------------------------------------------------------|-----------------------------|-----------|----------------------------------------------|
| Build Tooling                                                                                           | Provide Endpoints           | in        | provide / require / report (REST, API token) |
| Build Tooling                                                                                           | Require Endpoints           | in        | provide / require / report (REST, API token) |
| Build Tooling                                                                                           | Report & Validate Endpoints | in        | provide / require / report (REST, API token) |
| Web UI                                                                                                  | Report & Validate Endpoints | in        | fetch JSON, SSE/WS updates                   |
| Web UI                                                                                                  | Live Update Endpoints       | in        | fetch JSON, SSE/WS updates                   |
| Web UI                                                                                                  | Admin Endpoints             | in        | fetch JSON, SSE/WS updates                   |
| Web UI                                                                                                  | Auth Endpoints              | in        | fetch JSON, SSE/WS updates                   |
| Application Services › Provide, Require, Spec Query, Branches & Reports, Administration, Access Control | Provide Endpoints           | out       | use cases                                    |
| Application Services › Provide, Require, Spec Query, Branches & Reports, Administration, Access Control | Require Endpoints           | out       | use cases                                    |
| Application Services › Provide, Require, Spec Query, Branches & Reports, Administration, Access Control | Report & Validate Endpoints | out       | use cases                                    |
| Application Services › Provide, Require, Spec Query, Branches & Reports, Administration, Access Control | Live Update Endpoints       | out       | use cases                                    |
| Application Services › Provide, Require, Spec Query, Branches & Reports, Administration, Access Control | Admin Endpoints             | out       | use cases                                    |
| Application Services › Provide, Require, Spec Query, Branches & Reports, Administration, Access Control | Auth Endpoints              | out       | use cases                                    |

