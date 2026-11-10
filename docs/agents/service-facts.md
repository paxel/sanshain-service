# Service facts for agents

The facts an agent needs before proposing, asking or coding, each with the source that is the
authority. Loaded into every session through `CLAUDE.md`. Read the matching topic **and its
source** before asking the user or proposing anything; a question the sources answer is not a
question. When a change alters a fact here, the same commit updates it (`AGENTS.md`, Documentation
Upkeep). Glossary: `CONTEXT.md`. User docs: `site/` (built by `scripts/build-site.js`).

## Model

- Producer publishes complete specs (OpenAPI, AsyncAPI 2.x/3.x, gRPC/proto); Consumer requires
  single endpoints at an exact Pin. Roles, not entities: one system is usually both and is listed
  in each role. Tables say `services` (Producer) and `clients` (Consumer); only the repository
  layer uses those words. Wire names `producername`/`consumername` only (1.x aliases refused).
- Tooling = software calling Sanshain for a system (our five clients below, curl). Never "client" in
  domain text, never a domain entity. Sanshain never sees git; clients never read git.
- Version: strict `MAJOR.MINOR.PATCH` from the spec (`info.version`; proto: mandatory
  `// sanshain-version:` comment, 400 when missing or conflicting). Never assigned by Sanshain.
  Version line = one Producer × one API type.
- No branches since 2.0 (ADR-0003). Stability is declared per provide: `snapshot` (default) or
  `ga` (the client's ga switch, set by CI on release pipelines only). Clients: ga flag >
  `SANSHAIN_GA` (explicit false counts) > `autoGa: true` in `sanshain.yaml` with a declared stream
  (trunk or tag) > snapshot; the service never sees `autoGa`.
- Sources: `CONTEXT.md`, `docs/adr/0003-versions-replace-branches.md`, `site/start/concepts.html`.

## Version rules (provide)

- Snapshot: overwritable only by the user who last provided it; anyone else's different content
  is `409 snapshot_owned` (next free minor proposed); identical bytes from anyone are a no-op.
  (ADR-0003 still says last-writer-wins; the code and site changed that.) Never compat-checked.
  Expires when neither provided nor required for `snapshot_max_age_days` (30, 0 disables).
- GA: immutable; identical bytes no-op; different = `409 immutable_ga` + `proposed_version`. A GA
  claims its number forever (`409 snapshot_on_ga`). GA of a snapshot number = promotion in place
  (`promoted: true`; byte-identical keeps the snapshot's provider). Needs `release_ga`
  (`releaser` role), also for no-ops: `403 ga_requires_releaser`.
- Mislabel check: GA breaking vs. the highest GA below without a major bump =
  `409 breaking_without_major`. Removing an element deprecated in the previous GA is not breaking;
  the marker must sit on the removed element (no whole-spec switch).
- Normalized before anything (CRLF/CR→LF, BOM removed); hash and no-op compare the normalized
  text (`docs/adr/0008-ga-content-immutable-apart-from-line-endings.md`). Duplicate endpoints after path normalization: 400.
- Answer `202 {version, stability, content_hash, changes{inserts,updates,deletes}, promoted}`
  (+ `harvested_subscriptions` for AsyncAPI). `dry_run`: full checks, persists nothing, no
  webhooks; ga dry run still needs releaser.
- Retire: provide with `retired: true` and nothing else (body/stability/trunk/tag = 400); clears
  the family's capability tag, closes its trunk pins, releases AsyncAPI contracts; history stays.
  Needs releaser or a maintainer grant.
- Escape hatch: audited delete-version (`manage_producers`, Producer-scoped); frees the number.
- Code: `src/application/provide_service.rs`, `version_rules.rs`; splitting `src/openapi.rs`,
  `src/asyncapi.rs`, `src/proto.rs`.
- Sources: `site/developer/api-lifecycle.html`, `site/developer/sanshain-yaml.html`.

## Require and resolution

- Exact pin, no ranges/latest/default. One entry for the pinned version, GA else snapshot, no
  fallback, no waiting. Served (records the dependency; headers `x-sanshain-stability`,
  `x-sanshain-resolution`), Absent `410` (version lacks the endpoint), Unknown `404` (no such
  version or Producer). A failed require records nothing; resolution never creates entities.
- `POST /require-bundle`: several endpoints of one Producer at one pin, one merged document,
  shared schemas once; 410 names the missing endpoints.
- ETag + `If-None-Match` → `304`. `X-Sanshain-Deprecated: true` on deprecated endpoints.
- `format=openapi` on an AsyncAPI require: payload schemas as OpenAPI 3.0.3.
- Matching: OpenAPI path + verb (parameter names, double/trailing slashes normalized); AsyncAPI
  channel (2.x key, 3.x address) + `PUB`/`SUB`; proto fully qualified service + rpc
  (case-sensitive).
- Code: `src/application/require_service.rs`. Source: `site/developer/sanshain-yaml.html`.

## Origin (why a version exists)

- Only provides carry an Origin: `build_url`, `change_url` (PR/MR), `tickets` (≤20 ids),
  `note` (one line ≤200). Requires and catalog pushes carry none.
- It comes from the pipeline, never from a person. Clients fill the two URLs from runner
  variables: GitHub Actions (run URL; PR URL on `refs/pull/<n>/…`), GitLab CI (`CI_JOB_URL`; MR
  URL in MR pipelines), Jenkins (`BUILD_URL`; `CHANGE_URL` on multibranch PR builds). Bitbucket
  Pipelines, Azure, CircleCI, Buildkite, TeamCity fill nothing: set `SANSHAIN_BUILD_URL`/
  `SANSHAIN_CHANGE_URL`. Precedence: client flag > `SANSHAIN_*` env > runner variables. Tickets
  and note only from flags or `SANSHAIN_TICKETS`/`SANSHAIN_NOTE`.
- Latest provide carrying any field wins; a no-op re-provide only fills empty fields. The build
  link shows only from a trunk build (branch builds' links die after the merge).
- Shown on the version row, diff, blame; MCP answers; `version.published` webhook. Ticket ids link
  through the admin's ticket URL template.
- Sources: `site/developer/ci-integration.html` ("Why: the origin of a build"),
  `site/developer/api-lifecycle.html`, `CONTEXT.md` (Origin).

## Annotations (2.13.0)

- Project-declared `{scope, name, value}` entries from `sanshain.yaml`, filled by the build
  (placeholders `{{NAME}}`: Maven `-DNAME`, CLIs `--set NAME=value`, else `SANSHAIN_SET_NAME`;
  unfilled entry left out with a warning). Sanshain shows them, never interprets them.
- `scope: version` only on a provide (else 400), stored per version: a real write replaces the
  names sent, a no-op re-provide fills missing names only. `scope: system` on provide, require
  (query parameter `annotations`, JSON array), bundle, catalog push; stored per system, latest
  wins per name. Retire with annotations: 400. Dry run and failed calls store nothing.
- Limits: 20 per scope per call, name ≤40 `[A-Za-z0-9_-]`, value ≤2048, not blank, no duplicate
  name; errors 400 prefixed `annotations: `. Pre-2.13 services: body field 422, query 400; the
  clients retry without and warn.
- Deleted with the version (every origin-deletion path) or the system's last role; Clear system
  information deletes system ones. Shown as link chips (http(s)) or text on version row, node
  panel, Producer/Consumer pages; MCP `describe_service`; `version.published` carries the sent
  version annotations.
- Code: `src/application/annotation_service.rs`, `Annotations::from_entries` in
  `src/domain/models.rs`. Source: `site/developer/sanshain-yaml.html` ("Annotations").

## Clients (tooling)

- Ours: the same team builds the service and the five clients, from this Claude setup. Checked
  out next to each other under `/all_dev` (the host's development folder, mounted in the
  sandbox); `/all_dev/Sanshain` is a stale old monorepo copy, not used. Each is its own GitHub repo
  `paxel/<name>` with its own issues, CHANGELOG/OLDER_CHANGES and releases.

  | Repo (`/all_dev/…`)     | Version | Language, layout                                      | Checks                                  |
  |-------------------------|---------|-------------------------------------------------------|-----------------------------------------|
  | `sanshain-maven-plugin` | 2.5.0   | Java 11, `/all_dev/sanshain-maven-plugin/src/main/java/com/sanshain/maven/` (Mojos, `SanshainMojoDelegate` = precedence, `SanshainConfig` = yaml, `SanshainOrigin`, `SanshainHttpClient`, `SpecCombiner`); own `CLAUDE.md` | `mvn verify -B` (JUnit 5, WireMock)     |
  | `sanshain-rs`           | 4.0.0   | Rust workspace: `sanshain/src/lib.rs` (library), `cargo-sanshain/src/main.rs` (CLI) | `cargo test`, clippy                    |
  | `sanshain-go`           | 2.5.0   | Go: `cmd/sanshain-go/main.go`, `internal/{api,config,cache}` (`api/origin.go`); `ai/rules.md` | `go test ./...`, golangci-lint, gosec   |
  | `sanshain-js`           | 2.6.0   | TypeScript: `/all_dev/sanshain-js/src/{api,cli,config,cache}.ts`, `bin/`, `action.yml` (GitHub Action) | `npm run build`, `npm test` (jest)      |
  | `sanshain-conan`        | 2.5.0   | Python: `sanshainconan/{client,cli,config,integration,catalog,cache}.py`; `ai/rules.md` | `uv run python -m unittest discover tests`, flake8, black, bandit, mypy |

- Every client does the same steps: load `sanshain.yaml`, resolve settings by precedence (flag >
  env > file > default), resolve the Origin (flag > `SANSHAIN_*` > runner variables, e.g.
  `sanshain-go/internal/api/origin.go`), call provide/require/catalog; the Maven, Go, JS and Conan CLIs keep
  require ETags in a state file (sanshain-rs and the conanfile helper keep none). A client change usually lands in all five with the same rule text.
- All read `sanshain.yaml`: `serviceName`, `sanshainUrl`, `compression`, `bestEffort`,
  `catalogInfo`, `strict`, `autoGa`, `annotations[scope, name, value]`,
  `provides[file, apiType, retired]`,
  `requires[serviceName, version, outputDirectory, endpoints[method,path], apiType, format]`.
  No secrets; no version, no stability in the file.
- Env: `SANSHAIN_URL`, `SANSHAIN_TOKEN`, `SANSHAIN_GA`, `SANSHAIN_TRUNK`, `SANSHAIN_TAG`,
  `SANSHAIN_BEST_EFFORT`, `SANSHAIN_SET_<NAME>` (annotation placeholders), the origin variables.
  Strict mode fails on any `Sanshain-Warning`.
- Commands: provide, require, catalog. Dry run only Maven (`-Dsanshain.dry.run`) and JS CLI.
- The service keeps each client's name, repo, version, install line and commands as data in
  `src/application/setup_service.rs` (`CLIENTS`); `tests/site_clients_table_test.rs` holds it
  equal to the table in `site/developer/clients.html`. A client release updates both.
- Clients speak service 2.x (sanshain-rs is versioned on its own, 4.x); origin needs service
  ≥2.9.0, catalog push ≥2.4.0, annotations ≥2.13.0 (older: retried without, with a warning).
- The site is the authority for each client (#93): one page per client,
  `site/developer/clients/*.html` (maven, rust, go, javascript, conan), every option with the client
  version it arrived in; the client READMEs keep install, a minimal example and a link. A client
  change updates its page. The Action ref is `GITHUB_ACTION` in `setup_service.rs`
  (`paxel/sanshain-js@summer`, no `main` branch), checked across `site/` by the same test.
- Sources: `site/developer/clients.html`, `site/developer/sanshain-yaml.html`.

## Streams and graphs

- Per call one stream: none (dev), `trunk` (default-branch CI), `tag=<sanshain-branch>`
  (release/hotfix). Streams never change version rules or what is served.
- Dev graph: all activity, thinned by `dependency_max_age_days` (30). Main graph: trunk pins,
  last write per (consumer, producer, api type, endpoint) wins; trunk provides mark "trunk's
  current version" (`trunk_member_versions`). Stale after half `trunk_max_age_days` (90), closed
  after it. All graph writes append-only (`valid_from`/`valid_to`), so main and branches have a
  timeline; history is never deleted.
- Unknown tag, or trunk + tag: succeeds, dev only, `Sanshain-Warning` (ADR-0005 amendment).
  Branches never auto-created.
- Sanshain-branch: named copy of main (or another branch) at an instant (retroactive allowed),
  `POST /admin/branches` (`release_ga`, session only). `main`/`dev`/`trunk` reserved. Versions
  referenced by value: a deleted version dangles and heals on re-provide; deleting a referenced
  Producer/Consumer is `409`.
- Reports and MCP take `scope=dev|main[@instant]|<branch>[@instant]`; `dev@…` is 400.
- Code: `src/application/branch_service.rs`, `report_service.rs`; UI `static/js/graph.js`,
  `static/js/pages/graph-1.js`, `static/js/graph-domains.js`.
- Sources: `docs/adr/0004-trunk-flag-and-graph-views.md`, `docs/adr/0005-sanshain-branches-and-timeline.md`, `site/developer/release-graphs.html`.

## AsyncAPI specifics

- 2.x `publish`/`subscribe` read from the application's perspective (inverse of the spec text);
  3.x `send`/`receive`. Stored id: 2.x channel key, 3.x channel address.
- GA `publish` messages register message contracts per (channel, message name; else title):
  first publisher owns; others must match semantically (`409 message_contract_conflict`).
  Both read a message through local `$ref`s (also in `oneOf`); an external `$ref` stays unnamed.
- `subscribe`/`receive` are harvested on every provide as version-less consumer edges
  (`harvested_subscriptions`), inheriting the provide's trunk flag, not on tag provides;
  unfulfilled ones point at a dashed `<channel> ?` node. Drift blocks at GA (409), advisory on
  snapshots (ADR-0006).
- Sources: `docs/adr/0006-asyncapi-subscribe-harvesting.md`, `site/developer/api-lifecycle.html`, `src/asyncapi.rs`.

## Catalog push (Backstage)

- `PUT /catalog/{systemname}` `{catalog_info_yaml, trunk}`; reads only the Component named like
  the system: `spec.dependsOn` `resource:` refs → Peripheral Resources (lower-cased, default
  namespace rules); `component:` refs naming no known system → Missing components;
  `spec.owner`, `spec.system` (fills an empty domain, never overwrites an admin's),
  `metadata.description`, `metadata.tags` (≤20, focus `tag:<name>`), source-location annotation,
  `metadata.links` (≤20). `consumesApis`/`providesApis`/lifecycle ignored; edges between known
  systems are never read from Backstage.
- Trunk push replaces trunk and dev sets; plain push only dev. No TTL. Unreadable file: `200`,
  `recorded: false`, `Sanshain-Warning`, previous state kept.
- Clear system information: `DELETE /admin/systems/{name}/information` (`manage_producers`,
  Producer-scoped; a Consumer-only name needs an admin): closes current dev and trunk periphery,
  deletes the context and the system annotations, audited `CLEAR_SYSTEM_INFORMATION`; timeline and
  sanshain-branch copies keep theirs. Button on the Producer and Consumer pages.
- Code: `src/backstage.rs`, `src/application/catalog_service.rs`.
- Sources: `docs/adr/0007-periphery-from-backstage-catalog-info.md`, `docs/adr/0010-backstage-context-beyond-the-periphery.md`, `site/developer/sanshain-yaml.html`.

## Products (#76, ADR-0011)

- Envelope of systems by name, defined from outside; one system in many Products; grants nothing
  over members. Names unique among Products only (may equal a system's), no `/`, `,`, ≤100 chars.
- Members: one origin each, `manual` (known Producer/Consumer only) or `helm` (pasted umbrella
  `Chart.yaml`: `alias` else `name`, exact match, unknown names skipped and reported). Import
  applies directly in one transaction: listed → `helm` (claims `manual`), unlisted `helm` removed,
  `manual` kept; bad YAML / no dependencies = 400, nothing changed. No FK: a member whose system
  lost every role shows "not in Sanshain" and is live again when the name returns.
- Routes in `maintenance.yaml` under `/admin/products*`: reads any signed-in; create/delete
  global `manage_producers`; members/chart `RouteGuard::Product(ManageProducers)` (global or
  Product Maintainer); maintainers `manage_roles` (user or group, directory groups by DN live).
  `/auth/me` answers `maintains_products`. All writes audited (`*_PRODUCT*`). Not in `api.yaml`,
  reports or release cuts. Nuke database removes Products.
- UI: `static/products.html` (list, `?name=` page; controls gated by `data-needs-product`), nav
  link; graph focus `product:<name>` beside `tag:` in `static/js/graph.js` (members fetched from
  `GET /admin/products/{name}`, then members + neighbours).
- Code: `src/application/product_service.rs`, `src/helm.rs`, `src/application/authz.rs`,
  `src/presentation/handlers/products.rs`, `static/js/pages/products.js`. Sources:
  `site/operator/roles.html`, `site/developer/web-ui.html`.

## Authentication and permissions

- Modes (`auth_mode`): `disabled` (Off/maintenance: API 503, only admins sign in), `dev` (needs
  `ALLOW_INSECURE_DEV_MODE=true`; provide/require/catalog/MCP without token as `dev_user`),
  `local` (registration and auto-approve switches), `ldap`, `oidc`. Local accounts always sign in
  by password in ldap/oidc. Fresh instance starts Off unless `INITIAL_AUTH_MODE`. Initial admin
  password: `INITIAL_ADMIN_PASSWORD`, else random in `initial-admin-password` next to the SQLite
  DB (`/data` with PostgreSQL; `initial_admin_password_file` in
  `src/infrastructure/sqlite_snapshot.rs`), mode 600, deleted after its change; never printed.
- Humans: session (7 days, `POST /auth/login`). Builds/assistants: API token `san_…`, scope `full`
  or `read_only` (read_only refused on provide/require/catalog). `/admin/*` takes sessions only
  (API token → 401), except the SQLite backup download (full token). User managers issue tokens
  for service accounts (`for_user`).
- Ten permissions: manage_users, manage_roles, manage_producers, manage_consumers, view_audit,
  manage_settings, manage_auth_config, view_observability, run_destructive_operations,
  release_ga. Fixed roles: admin, user_manager, viewer, releaser, maintainer (Producer-scoped
  assignment, never global). admin/releaser grants are admin-guarded. Root = `SANSHAIN_ROOT_USERS`
  (config, unrevocable, holds future permissions too).
- Groups: native or LDAP (by DN, membership read live, cached
  `SANSHAIN_DIRECTORY_GROUP_TTL_SECS`). OIDC grants only via `admin_group`.
- LDAP settings: a changed form must carry the bind password again for Test Connection and save;
  only an unchanged form uses the stored one.
- Every `/admin/*` route declares a `RouteGuard` (Authenticated, Global, Producer, Product; test
  in `src/lib.rs`).
- Code: `src/domain/permissions.rs`, `src/application/authz.rs`, `auth_service.rs`,
  `sign_in_service.rs`, `src/presentation/middleware.rs`.
- Sources: `site/operator/authentication.html`, `site/operator/roles.html` (its token section
  predates service-account tokens), `site/reference/api-usage.html`.

## Webhooks

- Events only: `version.published`, `provide.rejected`, `version.deprecated`,
  `dependency.recorded`, `branch.cut`, `catalog.recorded` (+ `webhook.test`). Never admin/audit
  events.
- Outbox row written in the fact's transaction, fan-out decided then; worker delivers with
  retries (24 h cutoff). Generic URL + secret, HMAC-SHA256; body formats json/slack/teams.
  Receivers must be on the admin allow-list (`webhook_allowed_hosts`, empty refuses system hooks).
- Scopes: global (admin), Producer's, Consumer's (events of every Producer it requires).
- Code: `src/application/webhook_service.rs`, `webhook_delivery.rs`. Sources: `docs/adr/0009-webhooks-outbox-and-egress-allowlist.md`,
  `site/operator/webhooks.html`.

## MCP

- `POST /mcp`, stateless Streamable HTTP, API tokens only (read-only recommended), sessions 401.
  Tools: list_services, describe_service, find_endpoint, get_spec, get_graph, diff_versions,
  blame, endpoint_structure; setup: setup_guide (role, build_tool, ci → client, commands, token,
  variables, skeleton, from `CLIENTS`), draft_sanshain_yaml (the file from named provides/requires/
  catalog/autoGa/annotations, never stored). 60 calls/min/token, 64 KiB per answer (summary + hint
  above). Read-only; never users, settings, audit, observability.
- Setup answers carry `instance {url, derived}`: `PUBLIC_BASE_URL` (`McpConfig.public_base_url`),
  else `X-Forwarded-Proto`/`X-Forwarded-Host`/`Host` of the request (`derived: true`). Builds use a
  separate `full` token from `/account.html`; the MCP token stays read-only. Code:
  `src/application/setup_service.rs`.
- review_spec (service, api_type, content, stability default ga): `review_provide` = the provide
  as a dry run past the GA gate (no releaser needed; snapshot ownership as the token's user;
  `VersionConflict.rule` names the rule), plus findings: missing_version_marker,
  absolute_server_url, unnamed_message, removed_without_deprecation (endpoint level, vs. the
  previous GA, also under a major), missing_summary. Code: `src/application/review_service.rs`.
- Code: `src/presentation/mcp.rs`, `src/presentation/mcp/tools.rs`,
  `src/application/mcp_service.rs`. Source: `site/developer/mcp.html`.

## Administration and operations

- Admin page tabs: Sign-in, Users, General. Retention pass every `CLEANUP_INTERVAL_SECS`:
  snapshot expiry, dependency cleanup, trunk close, credentials, webhook log (30 d), CSRF tokens.
- Danger Zone (`run_destructive_operations`): nuke producers/consumers/users/database, each with a
  typed confirmation.
- Audit: every provide, require, catalog push and admin action, with actor and stream;
  `/audit.html` needs `view_audit`.
- External Producer flag (grey node), set by admin upload or per row.
- Backup: SQLite `GET /admin/backup/sqlite` (VACUUM INTO, full token of an admin); PostgreSQL
  `pg_dump`. Helm CronJob optional.
- Deployment: one binary, one port 3000, image `ghcr.io/paxel/sanshain-service:v<x>`, UID 1000,
  `/data`. Exactly one instance (StatefulSet, 1 replica; the chart refuses more). Migrations
  forward-only, run at start, fatal on failure. `/health`, `/ready`, `/version`, `/metrics`
  unauthenticated.
- Configuration: every env variable in `site/operator/configuration.html`; invalid values stop the
  start. Runtime settings in the DB via `/admin/settings/*`. Data caches hold entries ≤10 s.
- Sources: `site/operator/administration.html`, `configuration.html`, `deployment.html`,
  `backup.html`, `observability.html`, `troubleshooting.html`.

## Contracts and docs that must move with code

- `api.yaml`: Producer/Consumer API (provide/require/report/catalog…), ~25 Producers pin it.
  `maintenance.yaml`: `/admin/*` with each route's permission. Router tests in `src/lib.rs` fail
  when a route is in neither (or the allowlist).
- `CHANGELOG.md`: current version only, `## [X.Y.Z]` without date; older in `OLDER_CHANGES.md`.
- `site/`: user docs per release; site tests in `tests/site/`; tables of config and permissions are
  checked (`tests/site_config_table_test.rs`, `tests/site_permissions_table_test.rs`).
- Screenshots/walkthroughs: `just screenshots`, `just record-walkthroughs` (writes
  `site/evidence/`).

## Code layout

- DDD layers, enforced by `tests/architecture_boundaries_test.rs`: `src/domain/` (models, ports,
  permissions; no frameworks) → `src/application/` (one `*_service.rs` per use case area,
  re-exported by `services.rs`; `mock_repo.rs` is the test repository) → `src/infrastructure/`
  (sqlite/postgres repositories, parallel on purpose; `cached_repository.rs`; LDAP/OIDC; TLS;
  migrations per dialect, never edited once applied) → `src/presentation/` (thin Axum handlers
  `handlers/{api,admin,auth,roles,products,webhooks,pages}.rs`, `middleware.rs`, MCP). Router in
  `src/lib.rs`, startup in `src/main.rs`.
- Spec handling: `src/openapi.rs`, `src/asyncapi.rs`, `src/proto.rs`, `src/structure.rs`
  (structure view), `src/backstage.rs`, `src/helm.rs` (Product `Chart.yaml`).
- Frontend: static pages `static/*.html` + `static/js/` (vanilla, classic scripts, CSP: no inline
  JS); header/nav/footer from `static/js/site-chrome.js`; Tailwind built to `static/css/app.css`
  (committed). Askama templates `templates/` (landing, dashboard).
- Tests: Rust in `tests/*.rs` (Postgres via testcontainers), UI Playwright in `tests/ui/` (import
  `test` from `tests/ui/fixtures.js`; CI runs the files listed in `.github/workflows/quality.yml`),
  site in `tests/site/`. `scripts/itest.sh` against a running service.
- CI: `ci.yml` (build, tests, cargo audit, npm audit), `quality.yml` (fmt, clippy, tests,
  tarpaulin floor, UI), `codeql.yml` (rust, javascript-typescript, actions), `security.yml`
  (weekly: the two audits, grype on the released image; ignores in `.cargo/audit.toml`),
  `pages.yml` (site), `release.yml`, `sonar.yml` (SonarQube Cloud on `summer`).
- Branches: `summer` is main; release branches `release/X.Y`.
