# Sanshain Service Improvement Backlog

This document is a handoff for follow-up implementation agents. It lists concrete service risks, missing features, bugs, and quality improvements found during a source review. Keep every change small, tested, and aligned with the current DDD/hexagonal layout:

- Domain contracts and models stay in `src/domain/`.
- Use-case logic stays in `src/application/`.
- Database and external adapters stay in `src/infrastructure/`.
- Axum handlers and middleware stay thin in `src/presentation/` and `src/main.rs`.

Do not “fix everything” in one pull request. Pick one item, add tests, implement the minimum safe change, run the relevant verification, update `CHANGELOG.md` only when the change is user-facing, and update `docs/ai/plan.md` when task status changes.

> **See also:** [`ai/testing-findings-2026-07-23.md`](testing-findings-2026-07-23.md) — a separate,
> prioritized plan for 16 issues found during exploratory testing (favorites loss, client-branch
> navigation dead-ends, audit/observability noise, dark-mode contrast, and several feature
> requests). Fold those items into this backlog's numbering as they are picked up.

## Priority legend

### 6. AsyncAPI subscribe operations: harvest as requires, validate as expectations — REWRITTEN 2026-07-04

> **2.0 note (ADR-0003):** the branch-era mechanics below are stale — contracts are now keyed
> `(channel, message name)` (no branch), enforced on GA provides only, and there is no
> protected-branch fallback in resolution. The SUB-harvesting idea itself still stands; re-scope
> the steps to version lines before implementing.

**Original approach rejected.** The earlier version of this item ("store and serve SUB operations
as endpoints") contradicts Sanshain's model: **provide = the contract a service produces,
require = what it consumes.** Storing SUB operations as provided endpoints would create a second,
competing schema per channel (the producer's PUB plus each consumer's SUB copy, under different
endpoint keys that nothing unifies), and protected-branch enforcement on a consumer's SUB would
protect nobody — a consumer changing what it reads breaks no one downstream. Decided 2026-07-04:
**only PUB ever registers a contract; a SUB section is a declaration of expectation** — harvested
as a require, validated against the producer's contract, never authoritative.

**Problem (still valid):** `parse_spec_endpoints()` drops AsyncAPI `SUB` operations with only a
server-side log warning the provider never sees. Consumer relationships reach the dependency
graph only if teams remember to hand-declare requires in `sanshain.yaml`.

**Impact:** The dependency graph and KAFKA virtual node silently miss consumer edges; drift
between a consumer's local schema copy and the producer's contract is never detected.

**Depends on:** #19 and #20 below. Steps 1–2 need only the stored PUB endpoints and can land
before #20; step 3 (drift validation) needs the #20 contract store.

**Relevant areas:**

- `src/application/spec_service.rs` (`parse_spec_endpoints`, provide flow, require flow, missing-endpoint tracking)
- `src/asyncapi.rs` (SUB extraction, message/payload helpers from item #5)
- `src/presentation/handlers/api.rs` (`provide_asyncapi` response shape), `api.yaml`
- `static/js/graph.js` (KAFKA virtual node — should need no change, only tests)
- `docs/api-lifecycle.md`, `docs/sanshain-yaml.md`, client-plugin docs

**Implementation instructions:**

1. **Harvest requires from SUB operations.** On a successful AsyncAPI provide, for each SUB
   operation in the submitted document, create a dependency edge (same application-level path
   the `/require/asyncapi` flow uses — do not insert rows directly in the provide handler):
   client = the providing service, target = the service owning the PUB contract for that channel
   on the branch (protected-branch fallback like the require flow). If no PUB provider exists,
   accept and record via the existing missing-endpoint tracking so the graph shows unfulfilled
   consumption.
2. **Surface it in the response.** Extend the AsyncAPI provide response with a
   `harvested_subscriptions` list (channel names + resolved/missing status), replacing the
   log-only warning. Update `api.yaml` accordingly.
3. **Drift validation (after #20).** For each SUB message payload the consumer declares, if a
   PUB contract exists for `(branch, channel, message-name)`: check the *expectation is
   satisfiable* — every property the consumer declares must exist in the contract with a
   compatible type (recursive; new helper `check_expectation_satisfied(contract, expectation)`
   in `src/asyncapi.rs`, the role-swapped sibling of `check_schema_compatible`). Consumer
   expecting *less* than the contract is fine; expecting a property/type the contract does not
   guarantee rejects the **consumer's own provide** with `409` naming channel, message, and
   property. This failure mode only ever affects the consumer submitting the spec.
4. **Document the direction convention loudly.** Sanshain reads AsyncAPI 2.x
   `publish`/`subscribe` from the **application's perspective** (`publish` = this service
   publishes). Note explicitly that the official 2.x spec defines the keywords from the client's
   perspective (inverted) and that Sanshain deliberately uses the app-perspective reading,
   matching its 3.x `send`/`receive` mapping.
5. **Document the fetch-to-file workflow.** Consumers get the producer's snippet via the
   existing `/require/asyncapi` / `/require-bundle`; the client plugin writes it as a dedicated
   local file which the developer `$ref`s from their own spec (multi-file support for codegen).
   The consumer's authored SUB section shrinks to channel refs + expectations. Plugin changes
   live in the client repos; this item only documents the server contract.

**Validation:**

- Unit tests: SUB harvesting for AsyncAPI 2.x and 3.x documents; `check_expectation_satisfied`
  accept (subset), reject (extra property, type mismatch) cases with exact error messages.
- Application tests: provide-with-SUB creates a dependency edge and reports it in the response;
  no-PUB-provider case records a missing endpoint; drift rejection returns 409 for the consumer
  only.
- Graph/report test: SUB-derived edges feed the KAFKA virtual node like manual requires.
- Run `cargo test`; JS/UI checks only if graph code changes.

## P1 — AsyncAPI contract-model rework (decided 2026-07-04)

Context for the two items below: the `shared_contracts` mechanism was originally
"Multi-Publisher Conflict Detection (Problem 2)" (see `OLDER_CHANGES.md`) — multiple services
providing the same AsyncAPI channel, first provide = source, later changes checked against it.
Two later events defanged it: the collision fix scoped contracts to `(branch_name, service_id)`
(correct for service-scoped REST paths, collateral damage for globally-namespaced Kafka topics —
cross-service linkage died, `owner_service_id` became vestigial), and `check_compatibility` was a
no-op for AsyncAPI until 1.5.0. What remains today is only a per-service baseline check on
*feature* branches (breaking changes there require `force`, contradicting
`docs/api-lifecycle.md`). Decisions: rip the remainder out (#19) and rebuild multi-producer
safety as message-level channel contracts (#20), enforced on all branches. Sequence: #19 → #20 → #6.

### 19. Rip out the legacy shared-contract mechanism — DONE (1.5.0)

Removed entirely, as decided 2026-07-04: application logic (`skip_compat`, the endpoint-loop
upsert/check blocks, the delete-loop release, `get_shared_contract_info`,
`has_protected_branch_endpoints`), the port methods and `SharedContract`/`SharedContractInfo`
models, all four repository implementations (sqlite/postgres/cached/database dispatcher) plus
`MockRepo`, the `/admin/shared-contract` route + handler, the dead `renderSharedContract`
UI code and the `has_changes`/"modified" badge (field removed from `EndpointRecord` and
`ClientEndpointInfo` end-to-end), and the `shared_contracts` table via new checksummed-layout
migration `20260704000000_drop_shared_contracts.sql` (SQLite + Postgres; old migrations
untouched). `force` stays accepted for API compatibility but is a no-op on non-protected
branches and still `400` on protected ones (`api.yaml` descriptions updated). Behavior change:
breaking changes on non-protected branches are now always accepted — protected branches are the
only compatibility gate, matching `docs/api-lifecycle.md`. Covered by
`test_breaking_change_allowed_on_feature_branch_without_force` (unit),
`test_feature_branch_accepts_breaking_changes_without_force` (integration, replaces the old
Problem-2 test), rewritten `scripts/itest.sh` sections 8/10, and a `CHANGELOG.md` "Removed"
entry. The historical 20240502 scope migration and its upgrade tests remain untouched.

### 20. Message-level AsyncAPI channel contracts (multi-producer topics) — DONE (1.5.0)

Implemented as decided 2026-07-04. New `channel_message_contracts` table (migration
`20260704000001_channel_message_contracts.sql`, SQLite + Postgres) keyed
`(branch_name, channel, message_name)` with `owner_service_id` (FK → `services`,
`ON DELETE CASCADE`) and `payload_yaml`; `ChannelMessageContract` domain model; five port methods
(`get_channel_message_contract`, `upsert_channel_message_contract`,
`delete_channel_message_contract`, `list_channel_message_contracts`,
`delete_orphaned_channel_message_contracts`) across sqlite/postgres/cached/database dispatcher +
`MockRepo`. Public `asyncapi::extract_pub_messages` harvests **named** PUB messages only (identity
= message `name`, fallback `title`; channel = 3.x `address` / 2.x key; app-perspective
`send`/`publish`), alongside new `check_payload_compatible`, `payloads_equal`, and `analyze_impact`
helpers. Provide-flow hook `plan_channel_message_contracts` (`src/application/spec_service.rs`)
runs after the endpoint delete-loop and before commit — it checks on both dry-run and real
provides but writes only after `apply_spec_changes`: first provider owns; the owner may widen via
the payload checker (breaking → `409` `BreakingChange`); a different **live** service must match
the owner's payload exactly (else `409` `Conflict` naming the owner, channel, message); a contract
whose owner service no longer exists is taken over (covers SQLite's un-enforced FK cascade);
owner-dropped messages delete the row (protected-branch non-deprecated removals are already
rejected by the item-#5 per-endpoint check before the hook runs). Enforced on all branches;
`force` does not bypass. AsyncAPI SemVer impact is now classified (additions → minor, doc-only →
patch, removal/incompatible → major) and aggregated into the provide bump. The periodic cleanup
task calls `cleanup_orphaned_channel_message_contracts` (drops rows for branches no longer present
on any service). Covered by asyncapi unit tests (extraction 2.x/3.x, unnamed-skip, oneOf,
deprecated, payload compat, equality, impact classification), spec_service application tests
(register, owner-widen minor bump, owner-breaking-on-feature rejected, identical co-publish
accepted, divergent co-publish `409` naming owner, owner-drop clears row, unnamed → no contract),
a SQLite repository test module, and a Postgres testcontainers repository test. Documented in
`docs/api-lifecycle.md` §6, `docs/user-guide.md`, and `CHANGELOG.md`. **Item #6** (SUB harvesting
+ drift validation) builds on this store next.

## P1 — Correctness and missing feature gaps

### 7. Make protocol removal explicit and clear stale Kafka/gRPC/OpenAPI markers

> **2.0 note (ADR-0003):** the instructions below are written against the branch model
> (service/branch sync, protected-branch rules, shared contracts) and are obsolete as written.
> The underlying concern — stale protocol tags/edges when a Producer drops a protocol family —
> needs re-triage against version lines before any implementation.

**Problem:** Production services can remain marked as Kafka/messaging even after their current `sanshain.yaml` no longer contains an AsyncAPI provide. The current service write paths are protocol-specific (`/provide`, `/provide/asyncapi`, `/provide/grpc`) and `provide_spec_inner()` only deletes endpoints for the same `ApiType` as the submitted spec. If a later Sanshain YAML update omits an entire protocol family, no request necessarily tells the service to delete the old endpoints, shared contracts, dependencies, or auto-tags for that missing family. `provide_spec_inner()` also auto-adds `messaging` for AsyncAPI and `grpc` for proto, but there is no matching tag reconciliation/removal when those protocols disappear.

**Impact:** The UI and reports can show stale Kafka/gRPC/OpenAPI capability labels, stale AsyncAPI dependency edges, and the virtual `KAFKA` graph node for services that no longer publish or require async APIs. This makes production architecture views untrustworthy and can hide real cleanup work.

**Relevant areas:**

- `docs/sanshain-yaml.md` (`provide`/`provides` may omit protocols or omit provide entirely)
- Client/plugin code that reads `sanshain.yaml` and calls `/provide*` / `/require*`
- `src/presentation/handlers/api.rs` (`provide`, `provide_asyncapi`, `provide_proto`, `require`, `require_bundle`)
- `src/application/spec_service.rs` (`provide_spec_inner`, `parse_spec_endpoints`, shared-contract update/delete logic, auto-tagging)
- `src/application/report_service.rs` (AsyncAPI renders as `KAFKA` in isolation reports)
- `static/js/graph.js` (injects virtual `KAFKA` node from `api_type=asyncapi` dependency edges)
- `static/producers.html` (shows stored endpoint `api_type` labels)
- `src/domain/ports.rs` and `src/infrastructure/*repository.rs` for endpoint, dependency, shared-contract, and service-tag cleanup APIs

**Implementation instructions:**

1. First reproduce and pin down the current failure:
   - Provide a service with OpenAPI + AsyncAPI in a `provides` list.
   - Update the same service/branch with only OpenAPI, as if AsyncAPI was removed from `sanshain.yaml`.
   - Assert that old `AsyncApi` endpoints, `messaging` tags, graph/report Kafka edges, or requires remain today.
   - Repeat with proto removal and with all provides removed because a service may legitimately become consumer-only.
2. Define removal semantics at the application boundary instead of guessing in JavaScript:
   - A full Sanshain YAML sync must send the complete desired protocol set for a service/branch.
   - Protocols absent from that desired set must be removed for that service/branch, subject to protected-branch compatibility rules.
   - An explicitly empty desired set must remove all provided endpoint families for that branch; it must not leave the old service shape behind.
   - Per-protocol `/provide*` endpoints may stay patch-like for backward compatibility, but the plugin/admin full-sync path must not use patch semantics when the source of truth is a whole `sanshain.yaml`.
3. Add an application-layer full-sync use case such as `sync_service_specs_from_sanshain_yaml`:
   - Accept service, branch, desired specs keyed by `ApiType`, base version/hash, `force`, actor, and optionally desired `requires`.
   - Reuse existing parser and compatibility helpers for each provided protocol.
   - For every previously stored `ApiType` that is missing from the desired set, generate deletes using the same `SpecChange::Delete` path instead of directly deleting rows in handlers.
   - Keep protected-branch behavior safe: removing a protocol family is equivalent to removing its endpoints and should be rejected on protected branches unless the existing policy deliberately allows it.
4. Reconcile stale metadata and graph inputs after endpoint removal:
   - Remove `messaging` when a service has no remaining AsyncAPI provides/requires and no AsyncAPI dependency edges.
   - Remove `grpc` when a service has no remaining proto provides/requires.
   - Do not derive persistent capability tags only from historical writes; recompute them from current endpoints/dependencies or keep them as explicit user-managed tags separate from auto-derived tags.
   - Clean `shared_contracts`, dependency rows, missing endpoints, and cache entries that reference removed protocol families, or mark them clearly as historical if they must be retained.
5. Update client/plugin behavior that processes `sanshain.yaml`:
   - When `provides` loses an `asyncapi`, `openapi`, or `proto` entry, call the full-sync route with that protocol absent rather than simply skipping the old `/provide*` call.
   - When `provide`/`provides` is absent entirely, call the full-sync route with an empty provided set so Sanshain removes old provided endpoints for that service/branch.
   - Apply the same desired-state logic to `requires`: dependencies removed from YAML should be removed from current dependency graph data, not left as stale Kafka edges.
6. Add an admin-safe cleanup/backfill for production data:
   - Provide a dry-run report listing services/branches with `messaging` tags but no current AsyncAPI endpoints/requires, and services with `grpc` tags but no proto endpoints/requires.
   - Provide an idempotent cleanup command or admin route to remove stale auto-tags and stale protocol-family endpoints only when the current source spec/YAML confirms they are absent.
   - Log counts and service/branch names, but never log full spec bodies.
7. Document the contract clearly:
   - `docs/sanshain-yaml.md` must state that removing a protocol entry from `provides` removes that protocol family from Sanshain on the next full sync.
   - Explain the difference between patch-like `/provide*` endpoints and desired-state full YAML sync.
   - Add a `CHANGELOG.md` entry because this changes user-visible cleanup semantics and production graph behavior.

**Validation:**

- Add application tests for OpenAPI-only update after prior OpenAPI+AsyncAPI, AsyncAPI-only removal, proto removal, and empty-provides consumer-only sync.
- Add protected-branch negative tests proving complete protocol removal is rejected or requires the same approved override path as endpoint deletion.
- Add repository tests for deleting all endpoints of one `ApiType` without touching the remaining protocol families.
- Add report/graph API tests proving stale `AsyncApi` dependencies no longer inject `KAFKA` after YAML removal.
- Add migration/cleanup dry-run tests against fixtures with stale `service_tags` and stale protocol edges.
- Run `cargo test`; run JS/UI checks if graph or services UI code changes.

### 8. Make admin spec and endpoint editing discoverable and complete — OBSOLETE (2.0.0)

> 2.0.0: manual endpoint editing was removed deliberately — versions are immutable (ADR-0003);
> `static/edit.html`, `update_endpoint_manual`, and `POST /admin/endpoints/update` no longer exist,
> only the read-only `/admin/endpoint-yaml` remains (`src/lib.rs`).

> **2.0 note (ADR-0003):** branch/`base_version`/`force`/`/admin/endpoints/update` references
> below are 1.x; re-verify the current admin surface (`maintenance.yaml`) before implementing.

**Problem:** Admins can create or update a service by submitting a spec through the API, but the UI does not provide an obvious service/branch-level action for adding endpoints or editing/replacing the source YAML. There is an existing endpoint-level YAML viewer and hidden admin-only edit flow (`static/yaml.html` -> `static/edit.html` -> `POST /admin/endpoints/update`), but it is only reachable after an endpoint already exists and does not help an admin add the first endpoint to a newly-created service.

**Impact:** Admin users can create empty service shells or discover services in the UI, then get stuck because adding endpoints and replacing specs appears to require knowing the raw `/provide` API contract.

**Relevant areas:**

- `static/producers.html` branch and endpoint list views
- `static/yaml.html` admin-only `Edit` button for existing endpoints
- `static/edit.html` manual endpoint editor
- `src/lib.rs` routes for `/provide`, `/provide/asyncapi`, `/provide/grpc`, `/admin/endpoint-yaml`, `/admin/endpoint-versions`, and `/admin/endpoints/update`
- `src/presentation/handlers/api.rs` provide handlers
- `src/presentation/handlers/admin.rs` (`admin_update_endpoint`)
- `src/application/spec_service.rs` (`provide_spec*`, `update_endpoint_manual`)
- `src/domain/ports.rs` and repository implementations if persisted full-source-spec lookup is missing

**Implementation instructions:**

1. Keep the main write path spec-first: admins should add endpoints by uploading or pasting a complete OpenAPI/AsyncAPI/proto document, then reuse the existing `provide_spec*` application use cases so endpoint parsing, compatibility checks, protected-branch rules, version history, shared-contract updates, audit logs, and notifications stay consistent.
2. Add discoverable UI actions in `static/producers.html`:
   - On the services page, show an admin-only `Provide spec` or `Add service spec` button near the search/header area.
   - On each service branch page, show an admin-only `Replace branch spec` or `Add endpoints` button.
   - On empty branch/endpoint states, include a clear admin call-to-action instead of only showing “No endpoints found”.
3. Build the editor as a service/branch spec editor, not a one-endpoint-only editor:
   - Prefer a new page such as `static/spec-editor.html`, or extend `static/edit.html` only if it remains clear which mode is endpoint-level versus full-spec-level.
   - Fields should include service name, branch, API type, version/base version, tags if supported, `force` for protected-branch override only when allowed, and a large YAML/proto textarea.
   - For existing branches, load the latest full submitted source spec if available; if the repository only stores split endpoint snippets, first add a proper full-spec version/source-spec store instead of reconstructing a fake document from snippets.
4. Add or expose admin-safe backend routes that delegate to existing application services:
   - Either call the existing `/provide*` routes with normal authenticated/admin credentials from the UI, or add thin `/admin/specs/provide` wrappers that only translate payloads and then call `services::provide_spec_with_actor`.
   - Do not duplicate parsing or compatibility logic in handlers or JavaScript.
   - Keep handlers thin and put any new business decisions in the application layer.
5. Keep the current endpoint-level editor, but make its scope obvious:
   - In `static/yaml.html`, label the existing `Edit` action as `Edit endpoint YAML`.
   - In `static/edit.html`, warn that it edits one stored endpoint snippet and is not the preferred way to add new endpoints to a service.
   - If `update_endpoint_manual` can create a new endpoint when none exists, document that behavior and test it; otherwise keep it strictly as edit-existing and return a clear `404`/`400` for missing endpoints.
6. Apply admin security and UX guardrails:
   - Hide editing actions from non-admin users and enforce admin auth server-side.
   - Include CSRF handling through the existing `apiCall` helper.
   - Show compatibility/protected-branch errors returned by the backend without swallowing details.
   - Do not log full submitted spec bodies on UI or backend errors.
7. Update user-facing docs after implementation:
   - `README.md` and relevant `docs/*.md` should explain how admins add or replace service specs in the UI.
   - `CHANGELOG.md` should get a `[0.1.0]` entry because this is a user-facing admin workflow.

**Validation:**

- Add UI/API integration tests for an admin creating a new service/branch by pasting a spec, replacing an existing branch spec, and seeing new endpoints on `producers.html`.
- Add negative tests: non-admin cannot see/use write routes, invalid YAML/proto is rejected, protected-branch breaking changes are blocked unless the existing force rules allow them, and stale `base_version` handling remains safe.
- Add or update application tests proving the UI route reuses `provide_spec*` behavior and records audit/version history consistently.
- Run `cargo test`, `npx eslint static/js/` if JavaScript files are changed, and the relevant Playwright/UI checks if available.

### 9. Reconcile README claims with implemented functionality — DONE (2.0.0)

> 2.0.0: README was rewritten for the version-line model — plugins are linked as external
> ecosystem repos ("Clients & Plugins"), and the contract-safety/multi-protocol claims now hold
> (`check_compatibility` in `src/application/spec_service.rs` dispatches native compat checks for
> OpenAPI, AsyncAPI, and Proto).

**Problem:** The README advertises broad features such as client plugins, live graph, auditing, contract safety, and multi-protocol support. Some are present but basic; others depend on external repositories or are partial inside this service.

**Impact:** Users and follow-up agents may trust behavior that is incomplete or implemented only for one protocol.

**Relevant areas:**

- `README.md`
- `docs/*.md`
- `src/application/spec_service.rs`
- `src/presentation/*`

**Implementation instructions:**

1. Create a feature matrix covering OpenAPI, AsyncAPI, and Proto for provide, require, splitting, compatibility checks, graph, audit, and clients.
2. Mark partial behavior explicitly instead of implying complete parity.
3. Link external client repositories as ecosystem integrations, not service-internal features.
4. Add “known limitations” sections for compatibility and AsyncAPI subscribe handling until fixed.

**Validation:**

- Documentation-only review is enough unless code changes are made.
- Ensure links are valid and examples match actual API behavior.

### 10. Add request-size limits and parser resource limits — DONE (1.5.0)

`create_app` now applies `DefaultBodyLimit::max(state.max_body_bytes)`; the limit is configurable via `MAX_SPEC_BODY_BYTES` (default `DEFAULT_MAX_BODY_BYTES` = 4 MiB, raised from axum's previous implicit 2 MiB; invalid or `0` values fall back to the default) and oversized bodies are rejected with `413 Payload Too Large` at extraction time — the limit counts decompressed bytes, so compressed uploads cannot bypass it. Parser-level protection relies on `serde_yaml_ng`'s built-in recursion limit, which turns pathologically nested YAML into a parse error surfaced as `400 Bad Request`; this is pinned by `split_rejects_excessively_nested_yaml` (tests/openapi_split_tests.rs). Over/under-limit behavior is covered by `test_request_body_over_limit_is_rejected_under_limit_accepted` (tests/integration_test.rs). Documented in `docs/configuration.md`; `api.yaml` lists the `413` response for `/provide*` and `/require-bundle`.

### 11. Tighten browser security headers and remove inline-script dependency

**Problem:** The CSP allows inline scripts/styles and CDN script/style sources. That is convenient for static pages but weakens XSS protection.

**Impact:** Any HTML injection or compromised CDN path gets more dangerous.

**Relevant areas:**

- `src/presentation/middleware.rs` security headers
- `static/*.html`, `static/*.js`, `static/*.css`
- Admin dashboard pages

**Implementation instructions:**

1. Move inline scripts and styles into static files.
2. Prefer vendored static assets over runtime CDN dependencies for admin pages.
3. Replace `'unsafe-inline'` with nonces or hashes if inline code cannot be removed.
4. Add `frame-ancestors 'none'` and review all existing directives.

**Validation:**

- Add or update a test that asserts the CSP header does not contain `'unsafe-inline'` after migration.
- Manually load admin pages or run existing Playwright/UI tests if available.

### 23. Trunk flag and main/dev graph views (ADR-0004, designed 2026-08-07) — DONE (2.2.0)

**Problem:** The dependency graph is the union of every pin any branch's build recorded recently,
thinned by `dependency_max_age_days` — it cannot distinguish the trunk stream's pins from
development churn, and edges flicker with CI activity. See
[ADR-0004](../docs/adr/0004-trunk-flag-and-graph-views.md) for the settled design; implement it
as specified there, no re-litigation of the decisions. Tracker spec: issue #30.

**Impact:** The graph cannot be trusted as an architecture view; superseded pins linger until
expiry, rarely-built services vanish and reappear.

**Relevant areas:**

- `src/infrastructure/migrations/{sqlite,postgres}/` — new migration
- `src/domain/models.rs`, `src/domain/ports.rs` — trunk fields on version/dependency records
- `src/infrastructure/*repository*.rs` — LWW upsert for trunk pins, both backends
- `src/application/spec_service.rs` (provide/require paths), `report_service.rs` (graph payload)
- `src/presentation/handlers/api.rs`, `api.yaml` — optional `trunk` on provide body, require
  params, require-bundle body (optional field, non-breaking; absent = today's behavior exactly)
- `src/presentation/handlers/admin.rs`, `maintenance.yaml` — `trunk_max_age_days` setting
- `static/graph.html`, `static/js/graph.js` — view toggle, conflict + staleness highlighting
- `docs/sanshain-yaml.md` (`sanshain.trunk`), `docs/user-guide.md` (graph views), `CHANGELOG.md`

**Implementation instructions:**

1. Migration: add `trunk_provided_at TEXT NULL` to `spec_versions` (set on every trunk provide of
   that entry, NULL = never trunk-provided). New table `trunk_dependencies`, **append-only**
   (ADR-0005 builds its timeline on this — never overwrite or hard-delete rows): columns for
   client, service, api_type, normalized_path/path, method, the pinned **version by value** (not
   a `spec_versions.id` FK), `valid_from`, and `valid_to NULL`-able. The current trunk pin per
   (client, service, api_type, normalized_path, method) is the row with `valid_to IS NULL`; a new
   trunk require closes that row (sets `valid_to`) and inserts the new one. TTL culling closes
   rows the same way instead of deleting. Separate from `dependencies` so none of this disturbs
   existing dev-graph semantics.
2. Contract: optional boolean `trunk` (default false) on the provide JSON body, the require query
   parameters, and the require-bundle JSON body. Document in `api.yaml`; absent flag must leave
   every existing behavior byte-identical.
3. Provide path: when `trunk=true` and not a dry-run, stamp `trunk_provided_at` on the written
   version entry. No effect on version rules, stability, GA immutability, or the no-op check.
4. Require path: when `trunk=true`, additionally record the trunk pin per step 1's append
   semantics (close the open row if its pin differs, insert the new one; identical pin just
   refreshes the open row's timestamp). The normal dependency recording still happens.
5. Settings: `trunk_max_age_days` (default 90), stored like `dependency_max_age_days`, exposed
   via the same settings surface in `maintenance.yaml`; the cleanup job *closes* trunk rows past
   the TTL (sets `valid_to`, never deletes) and clears stale `trunk_provided_at` markers.
6. Report: extend the graph payload with the trunk data (each producer's newest trunk-flagged
   version; trunk edges with their pins and `last_required_at`). Keep the existing dev payload
   unchanged.
7. UI: main/dev toggle on the graph page. Main view: producers at trunk version, trunk edges;
   highlight (a) major-lag conflicts (edge pin's major < producer trunk major), (b) stale entries
   (timestamp older than a fraction — e.g. half — of `trunk_max_age_days`).
8. Tests: repository LWW upsert (both backends), provide/require flag handling incl. dry-run,
   TTL cleanup, report payload shape; contract checks for the new optional fields.
9. Client libraries (separate repos, tickets exist): send `trunk` from `sanshain.yaml`
   `sanshain.trunk=true` / CI variable.

**Validation:**

- Two trunk requires of the same (client, producer, endpoint) with different pins leave exactly
  one *open* trunk row (the newer) and one closed one (the older, history preserved), while
  `dependencies` keeps accumulating as today.
- A provide/require without the flag produces bit-identical rows and responses to current 2.2.0.

### 24. Sanshain-branches: named release graphs and the timeline (ADR-0005, designed 2026-08-07) — DONE (2.2.0)

**Problem:** The main graph (item 23) shows only current trunk truth. What a *release* consists
of, which releases contain a given producer version, and what changed between two graphs is not
answerable. See [ADR-0005](../docs/adr/0005-sanshain-branches-and-timeline.md) for the settled
design; implement as specified there, no re-litigation. Depends on item 23 (append-only trunk
storage is the timeline's substrate) — implement 23 first. Tracker spec: issue #30.

**Impact:** Release/deployment truth, vulnerability impact lookup ("which releases pin
`b@1.0.0`?"), and release diffing all remain impossible; the branch-cut workflow has no
representation.

**Relevant areas:**

- `src/infrastructure/migrations/{sqlite,postgres}/` — branch tables
- `src/domain/models.rs`, `src/domain/ports.rs`, `src/infrastructure/*repository*.rs`
- `src/application/spec_service.rs` (tag on provide/require), new branch service functions
- `src/presentation/handlers/api.rs`, `api.yaml` — optional `tag` on provide/require/
  require-bundle, mutually exclusive with `trunk` (both → `400`)
- `src/presentation/handlers/admin.rs`, `maintenance.yaml` — branch create/delete/list endpoints
- `static/graph.html`, `static/js/graph.js` — graph selector, timeline slider, diff view
- `docs/sanshain-yaml.md`, `docs/user-guide.md`, `docs/administration.md`, `CHANGELOG.md`

**Implementation instructions:**

1. Migration: `sanshain_branches` (id, unique live name, created_at, created_by, source, as-of
   date). Branch membership/edges reuse the item-23 append-only row shape (version **by value**,
   `valid_from`/`valid_to`) keyed by branch id — a branch has its own timeline exactly like trunk.
2. Create: `POST /admin/branches` `{name, source?, as_of?}` — RouteGuard `release_ga`. Source
   defaults to trunk, `as_of` to now; source may be another branch. Copies the source graph's
   state at `as_of` (rows open at that instant) as the new branch's initial open rows. Duplicate
   live name → `409`. Audited. The `maintenance.yaml` description must be complete enough that a
   DevOps release-cut script can be written against it alone (all parameters, defaults, error
   answers) — this endpoint's primary caller is automation, not the UI.
3. Delete: `DELETE /admin/branches/{name}` — admin permission (higher bar than creation, per
   ADR-0005). Frees the name. Audited.
4. Wire: optional `tag=<name>` on provide, require, require-bundle. `tag` + `trunk` together →
   `400`. Unknown tag → `404` with an instructive "a releaser must create it first" message.
   Tagged calls append to the branch timeline with item-23 semantics (provide marks the member
   version, require updates the pin).
5. Lifecycle: extend the delete-version dependents listing (UI confirmation + the dependents
   endpoint) with referencing branches. Dangling by-value references render highlighted in the
   branch graph and heal automatically when the version is re-provided. No hard blocks.
6. UI: graph-page selector (main / dev / each branch), timeline slider with change markers
   (closed-row `valid_to` dates) for trunk and branches, date-pinned rendering, and a diff view
   between any two (graph, date) selections. "Create branch here" from a timeline position, for
   releasers.
7. Report/API: expose branch listing and a branch's graph (current or at a date) for the UI; keep
   existing report payloads unchanged.
8. Tests: branch create from trunk-at-date (incl. retroactive after further trunk movement),
   create from another branch, duplicate name 409, unknown tag 404, tag+trunk 400, tagged
   require updates branch not trunk, delete-version warning includes branches, dangling reference
   heals on re-provide, permissions (releaser create / admin delete), audit entries.
9. Client libraries: extend the four trunk tickets — same mechanism sends `tag` from
   configuration for release-branch pipelines.
10. Rename: `PUT /admin/branches/{name}` `{new_name}` — admin permission, audited with old and
    new name, duplicate live name → `409`. Branch identity is the internal id, so membership,
    timeline and audit stamps survive a rename untouched.
11. Audit: provide/require audit entries record the declared stream (trunk / tag name / none) in
    a nullable column; branch create/delete/rename write their own audit entries; the audit page
    gets a stream filter.
12. Reports: `/report` (and the reports UI) accepts an optional scope — `dev` (default, byte-
    identical behavior when absent), `main`, or `<branch>[@date]` — feeding report generation
    from the selected graph state through the existing code path.
13. Legend: view-aware — each graph view lists only markers it can draw (dev: today's set; main:
    + stale, + major-lag; branch: + dangling reference; timeline: + change markers). "Missing"
    stays applicable everywhere (snapshot overwrite or delete+republish can orphan any pin).
14. Reverse lookup: version rows on the producers page show chips naming the sanshain-branches
    that contain that version.
15. Exports: graph PNG and report exports are stamped with view + branch name + date.
16. Metrics: `sanshain_branch_updates_total{branch}` counter and a branch-count gauge; surface
    on the observability page.

**Validation:**

- Create "R" off trunk at date D after trunk moved on: R equals the main graph as it was at D.
- A hotfix tagged require changes R's edge and nothing in main/dev; R's timeline shows the change.
- Deleting a version pinned by R warns naming R; re-providing the version heals R's reference.

### 25. Graph symbol semantics: messaging badge, Broker node, outdated severity — DONE (2026-08-08)

Step 3 landed as a neutral authored glyph rather than the official AsyncAPI logo: vendoring a
third-party mark is a licensing decision, and the BROKER rename deliberately stops the graph
asserting a vendor. Drop the official SVG into `static/images/` and swap the glyph if wanted.
Original text:

Independent of items 23/24 — implementable immediately, frontend-only except the vendored asset.

**Problem:** Every node touching an AsyncAPI dependency edge wears the 🛢️ messaging symbol:
`static/js/graph.js` pushes a `messaging` tag onto every *client* of an asyncapi dependency
(lines ~494-500) and links both edge ends to an injected virtual node named `KAFKA` (~507-530),
although server-side only actual AsyncAPI providers carry the tag. The barrel emoji reads as
"oil", and "KAFKA" asserts a broker Sanshain has no evidence for. Separately, the `outdated`
edge flag (line ~69) is binary — one patch behind and three majors behind look identical.

**Impact:** With one async producer and three consumers, all four nodes show the barrel; the
architecture view misstates who provides messaging. Breaking-level pin lag is indistinguishable
from trivial lag.

**Relevant areas:** `static/js/graph.js`, `static/graph.html` (legend), `static/images/`
(new vendored asset), `CHANGELOG.md`.

**Implementation instructions:**

1. Delete the client-tag derivation for `messaging`/`grpc` in `graph.js` (~494-505): node badges
   come from server-side `service_tags` only, so only actual AsyncAPI providers are marked.
   Async involvement of consumers stays visible as the edge's api-type.
2. Rename the virtual `KAFKA` node to `BROKER` (constant, label, and any `service_tags` seeding).
3. Replace the 🛢️ emoji (node symbols and legend) with the official AsyncAPI logo as a local
   vendored SVG under `static/images/` (CSP forbids remote assets; do NOT use the Apache Kafka
   logo — ASF trademark and a broker assumption we just removed).
4. Split the `outdated` flag into two tiers against the line's latest GA: `outdated` (behind
   within the same major) and `breaking-outdated` (major behind), distinct colors, separate
   legend entries and highlight filters.
5. Update the legend labels accordingly ("Missing dep."/"Missing svc." stay as-is).

**Validation:**

- Fixture with one AsyncAPI provider and three consumers of it: exactly one node carries the
  AsyncAPI badge; the virtual node renders as `BROKER`.
- A pin one patch behind latest GA renders `outdated`; a pin one major behind renders
  `breaking-outdated`; both filters highlight independently.

## P1 — Data integrity and operational reliability

### 12. Make spec updates transactional — DONE (2.0.0)

> 2.0.0: `upsert_spec_version` writes the version row and the wholesale endpoint replace in a
> single `begin()`/`commit()` transaction in both `src/infrastructure/sqlite_repository.rs` and
> `src/infrastructure/postgres_repository.rs`.

**Problem:** `provide_spec_inner()` performs multiple repository operations: ensure service/branch, tag updates, shared contract updates, endpoint inserts/updates/deletes, version updates, audit entries, and notifications. If one operation fails midway, the database can be left partially updated.

**Impact:** Consumers can see inconsistent endpoint/version/audit state after failures.

**Relevant areas:**

- `src/application/spec_service.rs` (`provide_spec_inner`)
- `src/domain/ports.rs` repository trait
- `src/infrastructure/sqlite_repository.rs`
- `src/infrastructure/postgres_repository.rs`

**Implementation instructions:**

1. Add a transaction abstraction to the repository port or create explicit service-level transaction methods.
2. Keep the application layer free of raw `sqlx` types; expose a domain/application transaction interface.
3. Commit only after all endpoint, version, shared-contract, and audit writes succeed.
4. Emit notifications only after commit.

**Validation:**

- Add a repository test with an injected failure after some writes and assert no partial state persists.
- Run tests for both SQLite and Postgres adapters if available.

### 13. Fix log buffer sizing to honor `LOG_BUFFER_SIZE` — DONE (2026-08-08)

**Problem:** `main.rs` reads `LOG_BUFFER_SIZE` and initializes buffers with that capacity, but `LogCaptureLayer::on_event()` uses a hard-coded `max_size` of `100` for every level.

**Impact:** Configuration is misleading; operators cannot increase or decrease captured log retention as documented/intended.

**Relevant areas:**

- `src/main.rs`
- `src/presentation/middleware.rs` (`LogCaptureLayer`)

**Implementation instructions:**

1. Add a `max_size: usize` field to `LogCaptureLayer` or per-level sizes if needed.
2. Pass `log_buffer_size` from `main.rs` into the layer.
3. Use that value in `on_event()` instead of `100`.
4. Reject or clamp `0` if an empty buffer would break the admin logs UI.

**Validation:**

- Unit-test the layer with a small buffer size and assert old entries are evicted at that size.
- Run `cargo test`.

### 14. Add cleanup for expired sessions and API tokens — DONE (2026-08-08)

**Problem:** There is periodic cleanup for stale branches, dependencies, and in-memory CSRF tokens, but expired sessions/API tokens can remain in persistent storage unless repository validation deletes them elsewhere.

**Impact:** Tables grow over time, admin views become noisy, and old auth data remains available for forensic leakage.

**Relevant areas:**

- `src/main.rs` cleanup task
- Auth repository methods in `src/domain/ports.rs`
- `src/infrastructure/*repository.rs`
- Admin token/session views if any

**Implementation instructions:**

1. Add repository methods `delete_expired_sessions(now)` and `delete_expired_api_tokens(now)`.
2. Call them from the existing cleanup task.
3. Ensure validation still rejects expired credentials even before cleanup runs.
4. Log counts only, not token/session values.

**Validation:**

- Repository tests for expired and non-expired records.
- Integration test proving expired token/session is rejected.
- Run `cargo test`.

### 15. Improve startup configuration validation — DONE (2026-08-08)

**Problem:** Several environment values are parsed with fallbacks. Invalid values silently become defaults in some cases, while invalid database/bind values fail loudly.

**Impact:** Operators can think a setting is active when the service ignored it.

**Relevant areas:**

- `src/main.rs` environment parsing
- `docs/administration.md`, `docs/troubleshooting.md`

**Implementation instructions:**

1. Centralize configuration parsing into a typed config struct.
2. Fail startup on invalid numeric/bool values instead of silently defaulting.
3. Log the effective non-secret configuration at startup.
4. Never log secrets such as database passwords, LDAP bind credentials, or token peppers.

**Validation:**

- Unit-test config parsing for valid, missing, and invalid values.
- Run `cargo test`.

### 26. Make the pin stores hold the invariants they claim (found 2026-08-07)

**Problem B — the append-only claim is not enforced against participant deletes.**
`trunk_dependencies`, `branch_dependencies` and `branch_member_versions` declare
`client_id`/`service_id` as `ON DELETE CASCADE` (sqlx enables `PRAGMA foreign_keys`, Postgres
enforces natively), so deleting a Consumer or Producer physically removes their rows from every
recorded graph — including the *closed* rows the timeline reconstructs from. The migration
comment states the opposite ("the same append-only shape … closed records are the timeline").

**Impact B:** Retiring a decommissioned service silently rewrites what past release cuts
recorded: "Release Maribou" loses those edges and its timeline changes retroactively. This is the
one thing the by-value version design exists to prevent — a deleted *version* leaves a visible
dangling reference, but a deleted *participant* leaves nothing at all.

**Note:** dropping the cascade alone fixes nothing. Every pin query inner-joins `clients`/
`services` by id, so an orphaned row is invisible to readers either way, and `ON DELETE RESTRICT`
would instead block a legitimate admin delete. The real options are (a) denormalize the
participant *names* into the pin rows so history stands on its own, or (b) soft-delete
participants and teach the joins to include tombstones. Both are migration + backfill + a rewrite
of every pin query on both backends — which is why this sits with Problem A: same tables, same
writers, one migration slot rather than two.

**Decision 2026-08-08 (supersedes the Note above for the branch case):** deleting a Producer or
Consumer that a sanshain-branch's recorded graph still references is **refused with a 409** naming
the branches, enforced in the application layer. This is a hard block, which the Note above and
ADR-0005's "warnings, not hard blocks" argued against — the owner chose it deliberately: a release
cut is a frozen record, and a warning the admin clicks through still rewrites it. Trunk presence
does **not** block, so Problem B's impact survives for the trunk timeline, which is why this item
stays open.

**Superseded mitigation (2026-08-07):** `admin_delete_producer`/`admin_delete_consumer` now
answer `{"deleted", "branches"}` and name the affected release graphs in the audit entry
(`list_participant_branch_references`), mirroring the delete-version warning. The admin is told;
the history is still lost.

**Problem A — DONE (2026-08-08; migration-collapse test added 2026-08-10).** The concurrent-writes
Postgres test from the Validation list is still missing — the race is now prevented by the
constraint rather than demonstrated, which is weaker evidence than the item asked for. A UNIQUE partial index now enforces one open record per
pin key on all three tables, and the writers insert with `ON CONFLICT … DO UPDATE` so the loser
of a race refreshes the winner's row. Original text:
`record_trunk_pins` and `record_branch_pins` refresh-or-insert with a
check-then-insert inside a transaction, but nothing in the schema enforces "at most one open
record per pin key". `idx_trunk_dependencies_open` is non-unique, and under Postgres READ
COMMITTED two concurrent requires for the same pin each see `rows_affected == 0` on the refresh
`UPDATE` (neither sees the other's uncommitted `INSERT`) and both insert.

**Impact A:** Duplicate open rows for one pin key: the main graph and reports draw the edge twice,
and a later re-pin closes both rows at once, breaking the invariant every reader assumes. SQLite
serializes writes so it does not reproduce — the two backends silently diverge.
`record_branch_member_version` has the same check-then-insert shape (its
`UPDATE`/`INSERT` pair was made transactional on 2026-08-07, which fixes the crash-between-writes
half but not this one).

**Relevant areas:**

- `src/infrastructure/sqlite_repository.rs`, `src/infrastructure/postgres_repository.rs`
  (`record_trunk_pins`, `record_branch_pins`, `record_branch_member_version`)
- `src/infrastructure/migrations/{sqlite,postgres}/`

**Implementation instructions:**

1. Read the three writers first and derive the exact key column set from the `UPDATE` predicates
   — the pin stores match on `normalized_path`, not `path`; the member store has no path at all.
   The index must match the predicate exactly or it will not prevent the race.
2. Write one migration per backend that (a) closes pre-existing duplicate open rows, keeping the
   newest per key, then (b) creates a UNIQUE partial index on the open rows
   (`... WHERE valid_to IS NULL`). Step (a) is not optional: the index creation fails on any
   deployment that already accumulated duplicates.
3. Convert the writers to rely on the constraint (upsert / `ON CONFLICT`) rather than the
   check-then-insert, so the race is resolved by the database rather than by timing.
4. Migrations are checksum-verified — expect `tests/migration_checksum_test.rs`,
   `upgrade_test.rs` and `upgrade_test_postgres.rs` to need attention in the same change.

**Validation:**

- Postgres test issuing two concurrent identical trunk requires, asserting exactly one open row.
- A migration test seeding duplicate open rows and asserting the migration collapses them.
- Run `cargo test` (the Postgres suites need Docker).

### 28. Release-graph correctness gaps found in review — DONE (2026-08-10)

All eight sub-items shipped 2026-08-08/09; the Validation debt was paid 2026-08-10: backend items
1/4/5 have integration tests, frontend items 2/3/6/7 have Playwright regressions (each proven to
fail with its fix reverted). Known remainder: 28.8's atomicity itself is untestable without fault
injection — the transaction is the fix, and the branch-delete behaviour is covered. Original text:

Independent defects sharing one theme: a scoped or reconstructed view still mixes in
present-tense or unfiltered data. Each is small; they are grouped because they are found and
verified together, not because they must ship together.

1. **Scoped reports carry today's trunk graph.** `generate_scoped_report`
   (`report_service.rs:93`) replaces only `dependency_graph`; `trunk_graph` and
   `trunk_stale_before` still come from `generate_report`'s present-tense fill. So
   `GET /report?scope=main@<past>` is stamped with a historical `scope_label` while carrying
   today's trunk edges — the masquerade `scope_label` exists to prevent. Fix: for a non-dev
   scope, either fill `trunk_graph` at the same instant or empty it, as the dev-only overlays
   already are.
2. **Main view injects unfiltered producer nodes.** `renderCustomGraph` (`graph.js:724`) adds a
   node per `trunkVersionMap` key, built from the unfiltered `report.services_detailed` *after*
   `getFilteredReport` narrowed the edges. A focus tag, Circular mode, or an unchecked protocol
   filter still draws every trunk-provided producer as an isolated node, so the filters look
   broken. Fix: intersect the injected nodes with the filtered edge set.
3. **Dangling/stale dash patterns are overwritten.** `graph.js:1239` unconditionally resets
   `stroke-dasharray` to `6 3` for missing/pubsub/messaging edges, after the stale (`3 4`) and
   dangling (`2 5`) patterns were set; the colour chain lets pubsub win too. A dangling AsyncAPI
   PUB/SUB pin renders identically to a healthy one. Fix: fold dangling/stale into the same
   decision chain that already resolves colour (the 2026-08-07 fix did this for colour only).
4. **`list_graph_change_dates` ignores member versions.** Both backends scan only
   `*_dependencies`, so a branch whose hotfix was a tagged Provide with no tagged require has an
   empty timeline — no slider marker for a change that happened. Fix: union
   `branch_member_versions` valid_from/valid_to.
5. **Bundle entries colliding after normalization are dropped from the pin store.**
   `record_pins` (`spec_service.rs:1223`) sends one param per raw endpoint, but the store keys on
   `normalized_path`; the second of two colliding entries hits the refresh branch and vanishes,
   while `record_dependencies_bulk` keeps both — dev and main graphs then disagree. Fix:
   deduplicate by normalized key before recording, or make the collision an explicit rejection.
6. **Timeline slider races itself.** `graph.js:444` binds `input` (not `change`) and fires the
   async `setTimelinePosition()` unsequenced, so on a slow link the last response to land wins
   and the graph can settle on an instant the user scrubbed past. Fix: sequence with a request
   token, or bind `change`.
7. **`runGraphDiff` has no 401 path.** `graph.html:662` awaits `apiCall` unguarded; `apiCall`
   throws on 401 and `graph.html` defines no `onSessionExpired`, so an expired session leaves the
   panel stuck on "Comparing…". `selectBranchView` (`graph.js:481`) only `console.warn`s, leaving
   the select showing a branch the view never switched to.
8. **SQLite `delete_branch` is not transactional.** `sqlite_repository.rs:748` issues the child
   deletes and the branch delete as separate statements; a failure between them leaves a branch
   whose graph is gone but whose name is still held. Postgres relies on the declared cascade and
   is atomic — the backends diverge.

**Validation:** each item needs its own regression test; items 1, 4, 5 and 8 are backend and
belong in `tests/trunk_graph_test.rs`, items 2, 3, 6 and 7 in the Playwright suite.

### 29. Audit stream identifies a branch by name, not id — DONE (2026-08-08)

Resolved by adding `audit_logs.branch_id` (no foreign key, so an audit row outlives the branch it
names), stamping it on tagged provides/requires, and resolving the `?stream=` name to an id at
read time. `stream` remains the label the row was written with, and stays the only marker for
`trunk`, which is not a branch. A name that resolves to no live branch still matches by label, so
a deleted branch's history remains reachable under its old name. Original text:

**Problem:** every tagged build stamps `audit_logs.stream` with the branch *name*
(`provide_common`/`require_common`), while `rename_branch`'s doc states "identity is the id:
membership, timeline and audit stamps survive". Membership and timeline do — they key on
`branch_id`. Audit stamps do not.

**Impact:** after renaming `rel-1` → `release-maribou`, `/api/audit/timeline?stream=release-maribou`
returns nothing from before the rename; the history is reachable only under a name that no longer
exists. Worse, `delete_branch` frees the name, so a new branch reusing it inherits the deleted
branch's audit stream and "who changed Release Maribou?" merges two different release cuts.

**Note — needs a decision before implementation.** The options are not equivalent:
(a) stamp `branch_id` and resolve names at read time — correct, but the audit filter is a
user-typed name and historical rows would need backfilling; (b) rewrite `stream` on rename —
cheap, but it *edits existing audit rows*, which is a deliberate integrity decision nobody should
take unilaterally, and it does not fix name reuse after delete; (c) refuse to free a name on
delete. Ask before implementing.

### 27. Diff graphs by the key the pin stores actually use (found 2026-08-07)

**Problem:** `diff_graphs` keys pins by the raw `path`, while the pin stores match open records
by `normalized_path`. A path whose parameter is respelled (`/users/{id}` → `/users/{userId}`)
normalizes to the same stored record — one pin whose version moved — but the diff sees two
different keys.

**Impact:** `GET /admin/graph/diff` reports such a change as a removal plus an addition instead
of a version change, so a release diff claims a dependency was dropped and a new one introduced.
Misleading exactly where the feature is meant to be authoritative.

**Relevant areas:**

- `src/application/branch_service.rs` (`diff_graphs`, the `key` closure)
- `src/domain/models.rs` (`TrunkPinInfo` — has no `normalized_path` field today)
- the pin queries in both repositories

**Implementation instructions:**

1. Decide the seam: either select `normalized_path` in the pin queries and carry it on
   `TrunkPinInfo` (skipped in serialization if it should not reach the wire), or normalize in
   the application layer using the same function the write path uses. Prefer the former — two
   normalizers will drift.
2. Key `diff_graphs` by the normalized path; keep the raw `path` for display so the rendered
   diff still shows what a human wrote.

**Validation:**

- Unit test: two pin sets differing only in parameter spelling and version produce one
  `pins_changed` entry and no add/remove pair.
- Run `cargo test`.

## P2 — Maintainability and quality

### 22. Observability audit panel shows only the last 30 rows, now shared with rejections — DONE (2026-08-08)

> **2.0 note (ADR-0003):** protected branches and the Branch filter no longer exist; refusals are
> now 409s from the version rules. The crowding concern stands, but re-check what is audited today
> before implementing.

**Problem:** The Database Audit Log panel on `observability.html` loads a fixed
`get_recent_audit_logs(30)`. Since 1.7.0 protected-branch rejections (`REJECTED_SPEC`)
are written to the same table, so a burst of refusals — a CI job retrying a breaking
change — pushes real changes out of that window.

**Impact:** An operator looking at the observability page can miss recent settings
changes or provides because refusals crowded them out. The data is not lost; only this
panel's view of it is truncated.

**Accepted deliberately** when rejection auditing was added (issue #8): the audit
timeline at `/audit.html` supports filtering by action type, Producer and Branch and is
the intended place to investigate refusals in depth.

**Relevant areas:**
- `src/presentation/handlers/admin.rs` — `get_observability_audit_logs`, the hardcoded 30
- `static/observability.html` — the panel and its heading

**Options:** raise the limit, paginate, or add an action-type filter to the panel.

### 16. Split large service modules into focused use cases — PARTIALLY DONE (2026-08-09)

Extracted so far: `version_rules.rs` (the ADR-0003 version rules as pure functions — no repository,
no I/O) and `require_service.rs` (the Require use case and its params). `spec_service.rs` went from
1711 to ~1295 lines and now holds the Provide flow plus the read/query functions. Splitting Provide
out is the remaining step; it is the most entangled part (contract planning, audit, stream
resolution) and deserves its own pass. Original text:

**Problem:** `src/application/spec_service.rs` is large and mixes provide, require, bundle, compatibility, versioning, tests, and shared-contract logic.

**Impact:** Future agents are more likely to introduce regressions because related behavior is hard to isolate.

**Relevant areas:**

- `src/application/spec_service.rs`
- Existing tests inside that module

**Implementation instructions:**

1. Extract modules such as `provide_service`, `require_service`, `compatibility_service`, and `shared_contract_service` under `src/application/`.
2. Keep public re-exports compatible through `src/application/services.rs` to avoid a broad handler rewrite.
3. Move tests next to the extracted logic.
4. Do not change behavior during the split; make this a refactor-only PR.

**Validation:**

- Run the full existing Rust test suite before and after the refactor.
- Run `cargo fmt` and `cargo clippy -- -D warnings`.

### 17. Add API-level tests for every documented endpoint group — OBSOLETE AS WRITTEN

> **2.0 note (ADR-0003):** the test matrix below is branch-era (`base_version`, protected-branch
> breaking change, service/branch lookups — none of which exist anymore), and the route surface is
> now contract-checked against `api.yaml`/`maintenance.yaml` in `src/lib.rs`. Redo the matrix from
> the 2.0 docs if API-level coverage gaps remain.

**Problem:** Unit coverage exists for several application helpers, but API-level behavior can drift from docs when handlers, middleware, and services interact.

**Impact:** Regressions in auth, CSRF, payload handling, and response formats may reach users.

**Relevant areas:**

- `tests/`
- `src/presentation/handlers.rs`
- `docs/api-usage.md`

**Implementation instructions:**

1. Build a test matrix from `docs/api-usage.md`.
2. Cover auth required/forbidden/success cases for each protected group.
3. Include negative tests for invalid API type, missing service/branch/path, invalid YAML, stale `base_version`, and protected-branch breaking change.
4. Use real auth/session/token paths in tests; do not add production bypasses.

**Validation:**

- Run all integration tests plus `cargo test`.

### 18. Add an architecture boundary check — DONE (2026-08-09)

**Problem:** The project relies on humans and guidelines to maintain DDD boundaries. A future change can accidentally import infrastructure types into domain/application code.

**Impact:** Architecture erosion makes the service harder to test and port.

**Relevant areas:**

- `src/domain/`
- `src/application/`
- CI scripts or `justfile`

**Implementation instructions:**

1. Add a lightweight script or test that fails if `src/domain` imports Axum, SQLx, LDAP, tracing subscriber, or infrastructure modules.
2. Add a similar check preventing `src/application` from importing `src/infrastructure` or raw SQLx types.
3. Wire the check into the normal verification command.

**Validation:**

- Add one intentional fixture or unit assertion if possible.
- Run the script and `cargo test`.

## Suggested implementation order

1. ~~Message-level AsyncAPI channel contracts (#20).~~ — DONE (1.5.0).
2. AsyncAPI SUB → requires + drift validation (#6; step 3 drift validation now unblocked by #20).
3. Protocol removal and stale Kafka/gRPC/OpenAPI cleanup (#7).
4. ~~Admin spec and endpoint editing workflow (#8).~~ — OBSOLETE (2.0.0).
5. ~~Transactional spec updates (#12).~~ — DONE (2.0.0).
6. CSP/static asset hardening (#11).
7. ~~README reconciliation (#9).~~ — DONE (2.0.0).
8. Operational cleanups and configuration validation (#13–#15).
9. Module split and architecture checks (#16–#18).

## Verification baseline for implementation agents

For code changes, use this default checklist unless the specific item says otherwise:

1. Add or update unit tests for domain/application logic.
2. Add or update integration tests in `tests/` for API-level behavior.
3. Run `cargo fmt`.
4. Run `cargo clippy -- -D warnings`.
5. Run `cargo test`.
6. If UI/static behavior changed, run the relevant JS/UI checks from the project docs or `package.json`.
7. Update `README.md`, `docs/*.md`, and `CHANGELOG.md` only when the implemented change affects users.
