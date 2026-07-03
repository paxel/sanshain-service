# Sanshain Service Improvement Backlog

This document is a handoff for follow-up implementation agents. It lists concrete service risks, missing features, bugs, and quality improvements found during a source review. Keep every change small, tested, and aligned with the current DDD/hexagonal layout:

- Domain contracts and models stay in `src/domain/`.
- Use-case logic stays in `src/application/`.
- Database and external adapters stay in `src/infrastructure/`.
- Axum handlers and middleware stay thin in `src/presentation/` and `src/main.rs`.

Do not “fix everything” in one pull request. Pick one item, add tests, implement the minimum safe change, run the relevant verification, update `CHANGELOG.md` only when the change is user-facing, and update `docs/ai/plan.md` when task status changes.

## Priority legend

- **P0+**: enforcement guardrails that make all other rules machine-checked; do before anything else.
- **P0**: security or data-loss risk; do first.
- **P1**: documented feature gap, correctness issue, or production hardening.
- **P2**: quality, maintainability, performance, or developer-experience improvement.

## P0+ — Enforcement guardrails against AI regressions (do before everything else)

This codebase is 100% AI-generated and will stay that way. A source review (2026-07-03) showed that rules existing only as prose in instruction files get violated over time (example: "`unwrap()` is strictly forbidden" — yet 73 production `unwrap()` calls existed at review time), while the one machine-enforced rule (the self-checking `all_admin_routes_have_auth_middleware` test in `src/lib.rs`) has held across all generations. Every project rule must therefore be enforced by lints, tests, or CI — not by instructions. These items are numbered E2–E10 (E1, feature-gating `MockRepo` out of release builds, is done — see the 1.5.0 CHANGELOG; they outrank items 1–18 below). E2 must precede E3; the rest are independent.

### E2. Remove the remaining production `unwrap()` calls

**Problem:** 9 production `unwrap()` calls remain (the 64 in `mock_repo.rs` were removed when E1 feature-gated it): 8 ETag-related in `src/presentation/handlers/api.rs` (around lines 275, 280, 324, 329, 373, 378, 446, 451: `HeaderValue::from_str(&etag).unwrap()`), 1 in `src/openapi.rs`.

**Impact:** Each is a panic path inside a request handler — a malformed value aborts the connection and violates the project's own no-panic policy.

**Implementation instructions:**

1. For each ETag site, restructure so the fallible conversion happens once and failure skips caching instead of panicking:
   ```rust
   if let Ok(etag_value) = HeaderValue::from_str(&etag) {
       if headers.get("if-none-match") == Some(&etag_value) {
           // existing 304 return
       }
       response_headers.insert("ETag", etag_value);
   }
   ```
   Adapt to each call site's exact shape; behavior with a valid etag must be unchanged.
2. Locate the single `unwrap()` in `src/openapi.rs` (`grep -n "unwrap()" src/openapi.rs`) and replace it with `?`, `match`, or a safe default.

**Validation:** `cargo test` passes; ETag/304 behavior covered by an integration test (add one if none exists: request with matching `If-None-Match` returns 304).

### E3. Enforce the no-panic policy with deny-lints (depends on E2)

**Problem:** `cargo clippy -- -D warnings` does NOT check for `unwrap()` — `clippy::unwrap_used` is opt-in and was never enabled. The written rule has no teeth.

**Implementation instructions:**

1. Add to `Cargo.toml`:
   ```toml
   [lints.clippy]
   unwrap_used = "deny"
   expect_used = "deny"
   panic = "deny"
   todo = "deny"
   unimplemented = "deny"
   dbg_macro = "deny"
   ```
2. Create `clippy.toml` in the repo root so the ~167 test-code unwraps stay legal:
   ```toml
   allow-unwrap-in-tests = true
   allow-expect-in-tests = true
   allow-panic-in-tests = true
   ```
3. Run `cargo clippy --all-targets -- -D warnings` and fix any remaining findings (there should be none in production code after E2; if lints still fire inside `#[tokio::test]` functions, wrap the enclosing module in `#[cfg(test)]`).

**Validation:** `cargo clippy --all-targets -- -D warnings` is clean; deliberately adding an `unwrap()` to a handler makes it fail.

### E4. Pin the Rust toolchain

**Problem:** CI uses `dtolnay/rust-toolchain@stable`, so every new Rust release can introduce new clippy warnings and break green builds at random times, unrelated to any code change.

**Implementation instructions:**

1. Determine the current toolchain (`rustc --version`, currently 1.95.0) and create `rust-toolchain.toml`:
   ```toml
   [toolchain]
   channel = "1.95.0"
   components = ["clippy", "rustfmt"]
   ```
2. In `.github/workflows/ci.yml`, `quality.yml`, and `release.yml`, replace `dtolnay/rust-toolchain@stable` with the pinned version (`dtolnay/rust-toolchain@1.95.0`) so CI and local builds agree.
3. Document in `AGENTS.md` under Build: toolchain bumps are deliberate, separate PRs.

**Validation:** CI green; `rustup show` in the repo picks the pinned version.

### E5. Enforce a coverage floor in CI (ratchet)

**Problem:** `cargo tarpaulin` runs in CI but with no threshold — the documented "80–90% coverage" target is prose-only. Actual coverage was ~64% at last measurement (May `cobertura.xml`), so the target is currently fiction and can silently decay further.

**Implementation instructions:**

1. Run `cargo tarpaulin --out Xml` locally and note the real current percentage.
2. In `.github/workflows/quality.yml`, change the Coverage step to `cargo tarpaulin --out Xml --fail-under <current - 2>` (e.g. `--fail-under 62`).
3. Add a ratchet rule to `AGENTS.md`: whenever coverage rises, raise `--fail-under` to (new value − 2) in the same PR. Never lower it.

**Validation:** CI fails if a PR drops coverage below the floor.

### E6. Self-enforcing contract test: router ↔ `api.yaml`

**Problem:** "Keep `api.yaml` in sync with the code" is prose-only. Nothing detects drift between the routes registered in `create_app()` (`src/lib.rs`) and the documented OpenAPI contract.

**Implementation instructions:**

1. Follow the existing pattern of `all_admin_routes_have_auth_middleware` in `src/lib.rs`: add a test (there or in `tests/`) that
   - parses `api.yaml` via `serde_yaml_ng` (already a dependency) and collects all `paths` keys;
   - extracts all `.route("...")` paths from `include_str!`-ed `lib.rs` source;
   - asserts every `api.yaml` path exists in the router, and every API route exists in `api.yaml`.
2. Maintain an explicit allowlist constant in the test for intentionally undocumented routes (pages like `/`, `/dashboard`, `/health`, `/metrics`, `/LICENSE`, `/version`, fragments, static fallback). Axum `{param}` syntax matches OpenAPI path templating, so string comparison works.
3. Failure messages must list the exact missing/extra paths.

**Validation:** Test passes; removing a path from `api.yaml` makes it fail with the path named.

### E7. Real readiness probe and SQLite/replica conflict fix

**Problem:** Both k8s probes hit `/health`, which returns a static `"OK"` without touching the database — a pod with a broken DB still receives traffic. Separately, `deploy/kubernetes/deployment.yaml` sets `replicas: 2` with a `ReadWriteOnce` PVC and SQLite as default: the second pod either cannot mount the volume or risks corrupting the SQLite file.

**Implementation instructions:**

1. Add an unauthenticated `GET /ready` route: run `SELECT 1` against the active pool (add a `ping()` method to the repository port and implementations); return 200 on success, 503 with a short JSON body on failure. Keep `/health` as-is for liveness.
2. Update `deploy/kubernetes/deployment.yaml`: `readinessProbe.httpGet.path: /ready` (liveness stays `/health`).
3. Change `replicas: 2` to `replicas: 1` in `deployment.yaml`, and add a comment plus a note in `docs/deployment.md`: scaling above 1 replica requires PostgreSQL (`DATABASE_URL=postgres://...`) and removal of the SQLite PVC mount.
4. Update `api.yaml` with the new `/ready` route (E6's test will enforce this).

**Validation:** Integration test: `/ready` returns 200 with a live DB; unit/integration coverage for the 503 path where feasible. `kubectl apply --dry-run=client -k deploy/kubernetes/` still validates.

### E8. Run Playwright UI smoke tests in CI

**Problem:** `tests/ui/*.test.js` exist but CI only runs eslint/prettier for JS — UI regressions are invisible.

**Implementation instructions:** In `.github/workflows/quality.yml`, extend the `js-quality` job (or add a job): `npm ci`, `npx playwright install --with-deps chromium`, build and start the service in the background with `INITIAL_ADMIN_PASSWORD` set (reuse the wait-for-health loop from `rust-quality`), then `npx playwright test tests/ui/smoke.test.js`.

**Validation:** CI job green; breaking a smoke-tested page fails the workflow.

### E9. Branch protection (manual, human step)

**Problem:** CI runs on every push, but nothing prevents merging (or pushing directly to) `master`/`release/*` with red CI.

**Implementation instructions (for the repository owner, not an agent):** On GitHub (`paxel/sanshain-service`), protect `master` and `release/*`: require the `CI` and `Quality` checks to pass, require PRs (no direct pushes), forbid force pushes. Mirror expectations for the sourcehut remote where possible (`.build.yml` already exists).

**Validation:** A test PR with a failing check cannot be merged.

### E10. Eliminate the 13 `#[allow(clippy::...)]` attributes, then deny them (lowest urgency in this section)

**Problem:** The project policy forbids `#[allow]`, yet 13 exist: 6× `type_complexity` (`src/infrastructure/{postgres,sqlite}_repository.rs`), 6× `too_many_arguments` (`src/application/spec_service.rs`, `src/domain/ports.rs`, `src/presentation/handlers/{admin,auth,api}.rs`), 1× `collapsible_if` (`src/presentation/handlers/admin.rs:908`).

**Implementation instructions:**

1. `type_complexity`: introduce named type aliases in `src/domain/ports.rs` (or the repository modules) for the offending signatures.
2. `too_many_arguments`: group parameters into small structs (e.g. a params struct per use case, as already done with `RecordDependencyParams`/`UpdateEndpointParams`).
3. `collapsible_if`: restructure the condition as clippy suggests.
4. Only after all 13 are gone, add `allow_attributes = "deny"` to `[lints.clippy]` in `Cargo.toml` so new ones cannot appear.

**Validation:** `grep -rn "#\[allow" src/` returns nothing; `cargo clippy --all-targets -- -D warnings` clean.

## P0 — Security and production safety

### 1. Make dev mode impossible to leave enabled accidentally

**Problem:** `services::get_dev_mode()` enables dev mode when `SANSHAIN_DEV_MODE=true` or the persisted `dev_mode` setting is `true`. `validate_csrf()` and auth middleware paths then allow protected requests without normal credentials. Startup logs warn loudly, but a bad environment variable or persisted setting can still expose all APIs.

**Impact:** A production instance can become unauthenticated by configuration drift.

**Relevant areas:**

- `src/application/auth_service.rs` (`get_dev_mode`, `set_auth_mode`, `ensure_dev_user`)
- `src/presentation/middleware.rs` (`validate_csrf`, auth middleware)
- `src/main.rs` startup warning and bind handling
- `docs/administration.md`, `docs/troubleshooting.md`

**Implementation instructions:**

1. Add an explicit production safety gate, for example `ALLOW_INSECURE_DEV_MODE=true`, that is required in addition to `SANSHAIN_DEV_MODE=true` or persisted `dev_mode=true`.
2. Fail closed by default: if dev mode is requested without the safety gate, return an error at startup or force dev mode off and log a clear error.
3. Keep test-only helpers behind `#[cfg(test)]`; do not add magic token/user bypasses in production code.
4. Document the exact local-only workflow for enabling dev mode.

**Validation:**

- Add tests proving dev mode is denied without the safety gate and allowed only with it.
- Add middleware tests proving protected endpoints still reject unauthenticated requests when dev mode is not fully enabled.
- Run `cargo test` and `cargo clippy -- -D warnings`.

### 2. Stop accepting API tokens in query strings

**Problem:** API token authentication supports bearer tokens, but any route or helper that accepts tokens from a query parameter risks leaking credentials through logs, browser history, proxies, referrers, and screenshots.

**Impact:** Long-lived API tokens can be unintentionally exposed.

**Relevant areas:**

- `src/presentation/middleware.rs`
- API handlers that parse `token`, `api_token`, or equivalent query parameters
- `docs/api-usage.md`
- Integration tests under `tests/`

**Implementation instructions:**

1. Require `Authorization: Bearer san_...` for API token authentication.
2. If backward compatibility is needed, implement a short deprecation window with a warning header and a changelog entry; otherwise remove query-token support immediately.
3. Make error messages generic (`401 Unauthorized`) and do not echo supplied token values.
4. Update docs and examples to use headers only.

**Validation:**

- Add integration tests: bearer token succeeds, missing token fails, query token fails.
- Search docs for query-token examples and replace them.
- Run `cargo test`.

### 3. Avoid logging submitted API specifications on parse errors

**Problem:** `parse_spec_endpoints()` logs full submitted OpenAPI, AsyncAPI, and proto content when parsing fails.

**Impact:** Specs can contain internal URLs, examples, schemas with sample secrets, or unreleased contract details. Parse errors can leak that data to logs and the admin log viewer.

**Relevant areas:**

- `src/application/spec_service.rs` (`parse_spec_endpoints`)
- `src/presentation/middleware.rs` (`LogCaptureLayer`)
- Admin log UI and documentation

**Implementation instructions:**

1. Remove full `content` from warning logs.
2. Log only safe metadata: service name, branch, API type, content length, hash prefix, and parser error.
3. If detailed diagnostics are needed, return them to the authenticated caller without storing the full spec in logs.
4. Consider adding log redaction for known sensitive field names before entries reach `LogCaptureLayer`.

**Validation:**

- Add a test that submits invalid content containing a fake secret and confirms captured/logged messages do not contain the secret.
- Run `cargo test`.

### 4. Harden API token storage and comparison

**Problem:** API tokens are generated with high entropy but stored as raw `SHA-256` hashes. Plain hashes are fast to brute-force if the database leaks, and normal equality comparisons may not be constant-time.

**Impact:** A database leak gives attackers a cheaper offline token-cracking target than necessary.

**Relevant areas:**

- `src/application/auth_service.rs` (`create_api_token`, `validate_api_token`)
- Repository token lookup methods in `src/domain/ports.rs` and `src/infrastructure/*repository.rs`
- Token migrations

**Implementation instructions:**

1. Prefer storing an HMAC-SHA-256 of the raw token using a server-side secret (`API_TOKEN_PEPPER`) rather than a plain hash.
2. Keep a `token_hash_version` column so old hashes can be migrated gradually.
3. Compare candidate hashes with a constant-time equality helper where feasible.
4. On successful validation of an old token, rehash it into the new format if the raw token is available.

**Validation:**

- Unit-test new token creation, validation, revocation, expiry, and old-hash compatibility.
- Integration-test authenticated API calls with new tokens.
- Run both SQLite and Postgres migration tests if available.

## P1 — Correctness and missing feature gaps

### 5. Implement compatibility checks for AsyncAPI and proto

**Problem:** `check_compatibility()` returns `Ok(())` for all non-OpenAPI API types. README/docs advertise multi-protocol support and protected-branch safety, but breaking-change detection is effectively OpenAPI-only.

**Impact:** AsyncAPI and gRPC/proto providers can break consumers on protected branches without being rejected.

**Relevant areas:**

- `src/application/spec_service.rs` (`check_compatibility`, protected branch flow)
- `src/asyncapi.rs`
- `src/proto.rs`
- `docs/api-lifecycle.md`, `docs/README.md`, `README.md`

**Implementation instructions:**

1. Define a minimal compatibility policy before coding:
   - AsyncAPI: removing channels/operations is breaking; changing message payload schemas incompatibly is breaking; adding new channels is non-breaking.
   - Proto: removing services/methods/messages/fields is breaking; changing field numbers or wire types is breaking; adding optional fields/methods is usually non-breaking.
2. Add parser-level helpers that produce comparable structures instead of string-only checks.
3. Wire the helpers into `check_compatibility()` and protected branch enforcement.
4. If full compatibility is too large, document unsupported cases and fail conservatively for risky changes.

**Validation:**

- Add unit tests for removed AsyncAPI channels, changed payload schemas, removed proto methods, changed proto field numbers, and safe additions.
- Add protected-branch integration tests for all three API types.
- Run `cargo test`.

### 6. Store and serve AsyncAPI subscribe operations instead of dropping them

**Problem:** `parse_spec_endpoints()` skips AsyncAPI `SUB` operations and stores only `PUB` operations, with a warning that subscribe channels must be declared as requires elsewhere.

**Impact:** The advertised AsyncAPI registry is incomplete and dependency graph data can miss important consumer/provider relationships.

**Relevant areas:**

- `src/application/spec_service.rs` (`parse_spec_endpoints`, require/provide flows)
- `src/asyncapi.rs`
- Domain endpoint model fields for path/method/API type
- API docs and client examples

**Implementation instructions:**

1. Decide how to represent AsyncAPI directions consistently (`PUB`, `SUB`, or actual `publish`/`subscribe`).
2. Store both operation directions with stable normalized keys.
3. Update require/bundle logic so consumers can request the direction they need.
4. Update graph/reporting code so publish/subscribe relationships are visible.

**Validation:**

- Unit-test splitting an AsyncAPI document with both publish and subscribe operations.
- Integration-test provide, list endpoints, require endpoint, and graph/report output.
- Run `cargo test`.

### 7. Make protocol removal explicit and clear stale Kafka/gRPC/OpenAPI markers

**Problem:** Production services can remain marked as Kafka/messaging even after their current `sanshain.yaml` no longer contains an AsyncAPI provide. The current service write paths are protocol-specific (`/provide`, `/provide/asyncapi`, `/provide/grpc`) and `provide_spec_inner()` only deletes endpoints for the same `ApiType` as the submitted spec. If a later Sanshain YAML update omits an entire protocol family, no request necessarily tells the service to delete the old endpoints, shared contracts, dependencies, or auto-tags for that missing family. `provide_spec_inner()` also auto-adds `messaging` for AsyncAPI and `grpc` for proto, but there is no matching tag reconciliation/removal when those protocols disappear.

**Impact:** The UI and reports can show stale Kafka/gRPC/OpenAPI capability labels, stale AsyncAPI dependency edges, and the virtual `KAFKA` graph node for services that no longer publish or require async APIs. This makes production architecture views untrustworthy and can hide real cleanup work.

**Relevant areas:**

- `docs/sanshain-yaml.md` (`provide`/`provides` may omit protocols or omit provide entirely)
- Client/plugin code that reads `sanshain.yaml` and calls `/provide*` / `/require*`
- `src/presentation/handlers/api.rs` (`provide`, `provide_asyncapi`, `provide_proto`, `require`, `require_bundle`)
- `src/application/spec_service.rs` (`provide_spec_inner`, `parse_spec_endpoints`, shared-contract update/delete logic, auto-tagging)
- `src/application/report_service.rs` (AsyncAPI renders as `KAFKA` in isolation reports)
- `static/js/graph.js` (injects virtual `KAFKA` node from `api_type=asyncapi` dependency edges)
- `static/services.html` (shows stored endpoint `api_type` labels)
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

### 8. Make admin spec and endpoint editing discoverable and complete

**Problem:** Admins can create or update a service by submitting a spec through the API, but the UI does not provide an obvious service/branch-level action for adding endpoints or editing/replacing the source YAML. There is an existing endpoint-level YAML viewer and hidden admin-only edit flow (`static/yaml.html` -> `static/edit.html` -> `POST /admin/endpoints/update`), but it is only reachable after an endpoint already exists and does not help an admin add the first endpoint to a newly-created service.

**Impact:** Admin users can create empty service shells or discover services in the UI, then get stuck because adding endpoints and replacing specs appears to require knowing the raw `/provide` API contract.

**Relevant areas:**

- `static/services.html` branch and endpoint list views
- `static/yaml.html` admin-only `Edit` button for existing endpoints
- `static/edit.html` manual endpoint editor
- `src/lib.rs` routes for `/provide`, `/provide/asyncapi`, `/provide/grpc`, `/admin/endpoint-yaml`, `/admin/endpoint-versions`, and `/admin/endpoints/update`
- `src/presentation/handlers/api.rs` provide handlers
- `src/presentation/handlers/admin.rs` (`admin_update_endpoint`)
- `src/application/spec_service.rs` (`provide_spec*`, `update_endpoint_manual`)
- `src/domain/ports.rs` and repository implementations if persisted full-source-spec lookup is missing

**Implementation instructions:**

1. Keep the main write path spec-first: admins should add endpoints by uploading or pasting a complete OpenAPI/AsyncAPI/proto document, then reuse the existing `provide_spec*` application use cases so endpoint parsing, compatibility checks, protected-branch rules, version history, shared-contract updates, audit logs, and notifications stay consistent.
2. Add discoverable UI actions in `static/services.html`:
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

- Add UI/API integration tests for an admin creating a new service/branch by pasting a spec, replacing an existing branch spec, and seeing new endpoints on `services.html`.
- Add negative tests: non-admin cannot see/use write routes, invalid YAML/proto is rejected, protected-branch breaking changes are blocked unless the existing force rules allow them, and stale `base_version` handling remains safe.
- Add or update application tests proving the UI route reuses `provide_spec*` behavior and records audit/version history consistently.
- Run `cargo test`, `npx eslint static/js/` if JavaScript files are changed, and the relevant Playwright/UI checks if available.

### 9. Reconcile README claims with implemented functionality

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

### 10. Add request-size limits and parser resource limits

**Problem:** Spec upload/splitting endpoints parse user-provided YAML/proto content. Large or deeply nested documents can consume CPU/memory.

**Impact:** A client can degrade service availability with oversized specs or pathological input.

**Relevant areas:**

- Axum route setup and middleware in `src/presentation/` / `src/lib.rs`
- `src/openapi.rs`, `src/asyncapi.rs`, `src/proto.rs`
- Configuration docs

**Implementation instructions:**

1. Add a configurable maximum request body size, with a safe default.
2. Add parser-level guardrails where libraries expose recursion/depth/size controls.
3. Return `413 Payload Too Large` for body limit failures and `400 Bad Request` for parser-limit failures.
4. Document `MAX_SPEC_BODY_BYTES` or equivalent.

**Validation:**

- Integration-test payload just under and over the limit.
- Unit-test parser failure handling for intentionally excessive nesting if practical.
- Run `cargo test`.

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

## P1 — Data integrity and operational reliability

### 12. Make spec updates transactional

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

### 13. Fix log buffer sizing to honor `LOG_BUFFER_SIZE`

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

### 14. Add cleanup for expired sessions and API tokens

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

### 15. Improve startup configuration validation

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

## P2 — Maintainability and quality

### 16. Split large service modules into focused use cases

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

### 17. Add API-level tests for every documented endpoint group

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

### 18. Add an architecture boundary check

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

0. Enforcement guardrails E2–E8 first (E2 before E3; E4–E8 independent; E9 is a manual owner step; E10 may be deferred). E1 (feature-gate `MockRepo`) is already done.
1. Dev-mode production safety.
2. Remove query-string API tokens.
3. Stop logging submitted specs on parse errors.
4. Request-size limits.
5. Non-OpenAPI compatibility checks.
6. Protocol removal and stale Kafka/gRPC/OpenAPI cleanup.
7. Admin spec and endpoint editing workflow.
8. Transactional spec updates.
9. AsyncAPI subscribe support.
10. CSP/static asset hardening.
11. Operational cleanups and configuration validation.
12. Module split and architecture checks.

## Verification baseline for implementation agents

For code changes, use this default checklist unless the specific item says otherwise:

1. Add or update unit tests for domain/application logic.
2. Add or update integration tests in `tests/` for API-level behavior.
3. Run `cargo fmt`.
4. Run `cargo clippy -- -D warnings`.
5. Run `cargo test`.
6. If UI/static behavior changed, run the relevant JS/UI checks from the project docs or `package.json`.
7. Update `README.md`, `docs/*.md`, and `CHANGELOG.md` only when the implemented change affects users.
