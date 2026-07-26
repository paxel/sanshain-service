# Testing-Round Findings & Fix Plan — 2026-07-23

Issues found during manual/exploratory testing of the running service (PostgreSQL backend).
This is a handoff plan for follow-up implementation agents. Follow the same rules as
`ai/improvements.md`:

- Keep the DDD/hexagonal layout: domain contracts in `src/domain/`, use-case logic in
  `src/application/`, adapters in `src/infrastructure/`, thin handlers in `src/presentation/`.
- **One item per PR.** Add/adjust tests, run `just check` (and `verify_all.sh` for UI/behavioral
  items), update `CHANGELOG.md` only for user-facing changes, tick this file when an item is done.
- Each item below is written to be mechanically implementable: symptom, root cause with file
  references, proposed change, and the test that proves it.

Where an item needs a product decision before coding, it is marked **DECISION NEEDED** — do not
guess; get the call first.

## Priority legend

- **P0** — data loss or broken core flows. Do first.
- **P1** — correctness / UX bugs that mislead or dead-end the user.
- **P2** — features, polish, and signal-to-noise improvements.

## Suggested order

1. #1 Favorites lost (P0, data loss)
2. #13 / #14 Client-branch navigation dead-ends (P1)
3. #3 / #4 "No history" on feature branches (P1 — reproduce first; likely folds into #13)
4. #6 Branch view audited as report + #9 no-op provide audited (P1/P2, quick wins)
5. #7 Observability log missing service/branch (P2)
6. #16 Dark-mode contrast (P2, larger UI pass)
7. Remaining feature requests: #2, #5, #10, #11, #12, #15 (P2)
8. Policy calls: #8 tokens in audit (P2, DECISION NEEDED)

---

## P0 — Data loss

### 1. Favorites disappear "after update" — NOT A BUG (resolved 2026-07-26)

**Resolution:** confirmed by the reporter to be an account switch, not data loss. Favorites are
per-`user_id` and rendered for the currently authenticated user, so signing in as a different user
shows that user's (empty) favorites while the original rows remain intact — exactly the
"session-identity mismatch, not deletion" case anticipated below. No code change needed. (The
read/write paths were verified consistent: both `get_favorites` and `add_favorite` resolve the same
`axum::Extension<User>.id`, `auth.rs:277`/`285`.)

**Symptom (original):** After an update, a user's starred favorites (services/clients) are gone.

**Storage:** `user_favorites` is keyed `(user_id, item_type, item_name)` with
`user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE`
(`src/infrastructure/migrations/{sqlite,postgres}/20240505000000_user_favorites.sql`) — favorites
are tied to a specific `users.id` row and cascade-delete with it.

**What the code review ruled out (do not chase these):** ordinary re-authentication does **not**
recreate the user. `login_with_provider` (`src/application/auth_service.rs:281`) does
find-then-create (`find_user` first, `create_user` only if absent), and `create_user` is a plain
`INSERT` (no upsert/replace, `sqlite_repository.rs:1456`). So a repeat login keeps the same
`users.id`, and favorites are **not** cascade-wiped by normal re-auth. The only user-row deletion
is the explicit `admin_delete_user` (`auth_service.rs:196`).

**Remaining candidate triggers (reproduce to pick one):**
- **Identity mismatch, not deletion (most likely):** favorites are per `users.id`, so if the
  session after the "update" resolves to a *different* identity than when they were set — e.g. the
  instance restarted into dev/anonymous mode, or the tester was on a different session/user — the
  rows still exist in the DB but the favorites query returns none for the current identity. Check
  what `user_id` the favorites read uses across the "update" (real user vs `DevMode/Anonymous`).
- **UI-side filtering:** favorites are name-keyed; if the favorites list is cross-referenced
  against the current service/client list and the item was momentarily absent, the DB row survives
  but the UI hides it. Check `static/js/discovery.js` / `services.html` / `clients.html`.
- **Admin delete+recreate** of the user (cascade) — only if that actually happened in the repro.

**Diagnostic step (do first):** reproduce the exact "update" the tester did; before/after, query
`user_favorites` directly and compare with the `user_id` the current session resolves to. That
single comparison distinguishes "rows gone" (deletion/cascade) from "rows orphaned to another
identity" (the likely case) from "rows present but UI-filtered".

**Proposed fix (choose after diagnosis):**
- If identity mismatch: make the favorites read resolve to the same stable identity, or re-key
  favorites to the stable username instead of `users.id`.
- If UI filtering: stop hiding favorited-but-currently-absent items (or show them as
  unavailable).
- If a delete/reset path drops them unintentionally: scope it so favorites survive where intended
  (mirror how "Protected branches preserved" already works in the nuke path).

**Test:** repository test proving favorites survive the reproduced trigger; integration test for
the identified path (identity resolution or UI filtering).

**Relevant areas:** `src/infrastructure/{sqlite,postgres}_repository.rs` (`*_user_favorite*`,
user upsert/create), the admin reset/nuke handler in `src/presentation/handlers/admin.rs`,
`static/js/discovery.js` / `static/services.html` / `static/clients.html` (favorites rendering).

---

## P1 — Correctness / UX bugs

### 13. Client-only branch → clicking an endpoint fails with `Failed to load versions: 404` — PARTIALLY DONE 2026-07-23

**Done (falls back to the protected branch, per maintainer direction):** rather than a "not
available" dead-end, the history view now resolves the endpoint with a branch fallback.
`get_endpoint_version_history` (`src/application/spec_service.rs`) uses `find_endpoint_with_fallback`
(the same resolver the YAML view already used): try the requested branch, else the service's
configured fallback branch, else any protected branch. So a client-required branch the server only
publishes on `master` now shows `master`'s real history. `yaml.html` labels it ("Branch X has no
published spec — showing history from master") by comparing the served `branch_name` to the
requested branch. Only when **no** branch has the endpoint does the explicit "not available" state
show. Tested by `version_history_falls_back_to_protected_branch` (sqlite); JS syntax-checked.

**Still open (needs live reproduction):** *why* the services overview lists a service with
clickable branches that have no provided endpoints in the first place — i.e. whether `require`
creates service/branch/endpoint rows that surface as clickable-but-unprovided. With the fallback
above, clicking such a branch now shows the protected branch's spec instead of an error, so this is
lower urgency; still worth confirming against a running instance whether those branch cards should
appear at all.

**Original analysis (retained for context):**

**Symptom:** A client requires a service+branch the server has no *provided* spec for. The services
view still shows an entry for that service listing **only the branches the client uses**; clicking
an endpoint navigates to `yaml.html` and errors with *"Failed to load versions: Fetch error: 404"*.

**Root cause:** `yaml.html` calls `/admin/endpoint-versions?...`; when the endpoint does not exist
on that branch the service returns `404 NotFound` (`get_endpoint_version_history` →
`AppError::NotFound`, `src/application/spec_service.rs:752`). The 1.5.1 fallback only handles the
**empty-200** case (`res.length === 0`), not a 404 — so the fetch throws and lands in the catch
that prints "Failed to load versions". The card was clickable because the client view marks it
`resolved`/clickable even though the server side is `pending` (no provided spec).

**Proposed fix:**
- Frontend: in `loadCurrentSpecFallback` / the initial fetch, treat a `404` from
  `endpoint-versions` the same as empty — try the current-spec fallback, and if that is also 404,
  render a clear "This branch/endpoint has not been provided to the server yet" state instead of a
  raw fetch error.
- Data: only render an endpoint card as clickable when the server actually has a provided spec
  (`yaml_content` present) — a purely client-required, unprovided endpoint should show `pending`
  and not link into `yaml.html`.

**Test:** Playwright/UI test: client with an unprovided branch → endpoint shows `pending`, does not
dead-end; deep-linking `yaml.html` to a non-existent branch shows the friendly empty state, not
"Failed to load versions".

**Relevant areas:** `static/yaml.html` (`loadEndpointVersions`, `loadCurrentSpecFallback`),
`static/clients.html` / `static/services.html` (clickability gating).

### 14. Browsing a client's branch, the "→ server" link shows the same page — PARTIALLY DONE 2026-07-23

**Done:** `navigateToServiceEndpoint` (`static/clients.html`) dropped `api_type`, so the "resolved →
service" badge always opened the endpoint as OpenAPI (wrong viewer for AsyncAPI/proto, and a likely
contributor to the "same page" impression). It now passes `ep.api_type` through to `yaml.html`.

**Still open (needs live reproduction):** confirm whether the badge should open the *endpoint*
(current behavior) or the *service/branch* overview, and whether the provider branch differs from
the client branch (see below). Reproduce and decide the intended destination.

**Original analysis (retained for context):**

**Symptom:** From a client's branch view, clicking the link to the server ("resolved → service")
navigates but lands back on what looks like the same page.

**Root cause (to confirm):** `navigateToServiceEndpoint(service, branch, path, method)` in
`static/clients.html` builds `/yaml.html?...` **without `api_type`**, and the `resolved` badge uses
`ep.branch` which — for a client require — is the *client's* branch, not the branch the server
provided on. The target likely resolves to an endpoint that doesn't exist (see #13) or to the same
client context. Verify the branch passed is the **server/provider** branch, not the client's.

**Proposed fix:** pass the correct provider service + branch (+ `api_type`) into the link; if the
provider branch differs from the client branch, resolve it server-side (the dependency resolution
already knows which provider branch satisfied the require). Reuse one link-builder helper across
`clients.html` and `services.html` instead of the current duplicated inline builders.

**Test:** UI test: from a client branch, the server link opens the *provider's* endpoint page and
renders its spec.

### 3 & 4. Feature branches always show "No history" — even a branch cut from master with no changes — RESOLVED 2026-07-26

**Symptom:** Viewing an endpoint on a feature branch shows no version history. A branch created
from `master` with identical content also shows "no history", which is confusing because `master`
has history.

**Root cause:** version rows (`endpoint_versions`) are only written when the branch is protected
(`apply_spec_changes`, both backends). Feature branches never accumulate their own history.

**Traced without needing a live repro** (the earlier plan's options (b)/(c)/(d) were superseded by
reading the actual code path): this turned out to be a genuinely different bug from #13, not the
same one. `find_endpoint_with_fallback` (used by #13's fix) only checks whether the endpoint
*exists* on the requested branch — if a feature branch genuinely holds the endpoint (e.g. it was
branched from `master` with identical, unchanged content, so a real row exists there), the resolver
returns that branch's own endpoint immediately, without ever checking whether it has any version
*history*. `get_endpoint_versions` on that endpoint then legitimately returns empty, because only
protected branches ever write those rows. This is (b)-shaped but not the #13 case: existence is not
the problem, empty *history* despite existence is.

**Fix (option (A), now justified — this is exactly what (A) was for):** `get_endpoint_version_history`
(`src/application/spec_service.rs`) now: (1) checks the requested branch's own history first, and
returns it immediately if non-empty (protected branches — the fast, unchanged path); (2) if empty,
searches the service's fallback branch then every protected branch (via a new shared
`fallback_branch_candidates` helper, factored out of `find_endpoint_with_fallback` so both stay in
sync) for one with real history, returning the first found; (3) only if nobody has real history
does it fall through to the existence-based `find_endpoint_with_fallback` (the #13 path — covers a
branch that doesn't have the endpoint at all). `yaml.html`'s existing fallback-note UI (added for
#13) needed no change — it already compares the served `branch_name` to the requested branch.

**Test:** `version_history_falls_back_when_local_branch_has_no_history_of_its_own` — a feature
branch is given the endpoint's real (identical) content via a normal provide, confirmed to have its
own row, and history is asserted to be inherited from `master`'s real multi-version history, not
reported empty. `version_history_falls_back_to_protected_branch` (the #13 case) still passes
unchanged.

**Relevant areas:** `src/application/spec_service.rs` (`get_endpoint_version_history`,
`fallback_branch_candidates`, `find_endpoint_with_fallback`).

---

## P1/P2 — Audit & observability signal-to-noise

### 6. Merely viewing a branch is audited as "Generated report" — DONE 2026-07-23

**Symptom:** Opening a branch in the UI produces an audit entry like *"Generated report for
branch 'x'"*.

**Root cause:** `report`, `report_markdown`, `report_isolation`, `report_merged`
(`src/presentation/handlers/api.rs:~500-590`) each call `record_audit_log` with `action: "REPORT"`,
`action_type: "READ"`. The branch view in `static/services.html:269` fetches `/report?branch=…`
just to render, so every view logs a REPORT.

**Proposed fix:** stop auditing interactive/read report generation. Options: drop the audit call
from the plain `report` (JSON) endpoint used for viewing; keep audit only for explicit *exports*
(markdown/isolation/merged downloads) if those are considered deliberate actions; or filter
`action_type == "READ"` REPORT rows out of the audit **timeline** while keeping them for
read-access analytics. Prefer: don't audit the JSON `report` view at all.

**Test:** integration test: `GET /report?branch=…` does not add an audit-timeline row; an explicit
export still does (if kept).

### 9. A `provide` with no changes still creates an audit entry — DONE 2026-07-23

**Symptom:** Re-providing an unchanged spec logs `PROVIDE_SPEC … (version N, changes: +0, ~0, -0)`.

**Root cause:** the provide handlers audit unconditionally (only gated on `dry_run`), regardless of
`res.changes` (`src/presentation/handlers/api.rs:~87-105` and the AsyncApi/proto equivalents).

**Proposed fix:** skip the audit write (or downgrade to a non-timeline log) when
`inserts + updates + deletes == 0`. Apply consistently to OpenApi, AsyncApi, and proto provides.

**Test:** integration test: two identical provides → exactly one `PROVIDE_SPEC` audit row.

### 8. Token create/revoke in the audit log — RESOLVED 2026-07-26 (keep events, strip identifying content)

**Original symptom/question:** the audit timeline shows `CREATE_TOKEN` / `REVOKE_TOKEN` rows,
questioned by the tester. On follow-up, the actual objection was narrower than "should these events
be logged at all": the *ID* logged with each event was a raw internal UUID with no benefit — it was
never shown anywhere in the UI (the token list never displays it; it's only used internally as a
JS `onclick` parameter for the Revoke button), so a human reading the audit log gained nothing from
it, and `REVOKE_TOKEN` logged *only* the ID, no name, making it uncorrelatable to anything.

**Decision (maintainer, direct correction):** don't add more identifying detail to fix that — strip
identifying detail entirely. Keep the `CREATE_TOKEN`/`REVOKE_TOKEN` events (they still mark that
token lifecycle activity happened, at the right timestamp/actor), but log **generic text with no
token name and no ID**: `"Created an API token"` / `"Revoked an API token"`. Audit log entries are
permanent and visible to every authenticated user (see #18 below — the audit log is not actually
admin-restricted), so a token name its creator considered private — or the
opaque ID — has no way to be redacted after the fact once logged; better to never log it. (An
intermediate version of this fix that added the token's *name* alongside dropping the ID was
explicitly rejected for the same reason — it made the identifying-content problem worse, not
better.)

**Also fixed while touching this code:** `revoke_api_token`/`delete_api_token` previously returned
a bare `bool` that the handler never even checked — revoking a nonexistent or already-revoked token
silently returned `200 OK` and wrote a false "Revoked" audit entry regardless. `delete_api_token`
(`src/domain/ports.rs` and both backends) now returns `Option<String>` (the deleted token's name,
used only for the not-found check, never logged) and the handler returns `404` when nothing was
actually revoked.

**Test:** `revoke_token_404s_when_not_found_and_audit_has_no_identifying_details` — creates a real
token via the HTTP API, revokes it (200), revokes it again (404), and asserts neither audit entry's
`details` text contains the token's name or its ID.

### 7. Observability log entries always missing `service` and `branch` — DONE 2026-07-26

**Symptom:** the observability log viewer conveys no service/branch context; log lines are plain
`sanshain_service: …` messages.

**Root cause, confirmed by reading the actual code (corrects the original hypothesis in two ways):**
1. The raw log viewer (`renderLogs()` in `static/observability.html`, backing
   `/admin/observability/logs`) has **no service/branch columns at all** — it renders only
   timestamp/level/target/message. The original "columns that are always empty" framing was
   inaccurate; there was nothing to populate because there was nothing there.
2. `LogVisitor` (`src/presentation/middleware.rs`), which turns a `tracing::Event` into a
   `LogEntry` for the in-memory buffer, only ever captured the field literally named `"message"` —
   every other field an event might carry was silently discarded.
3. Independently, the provide path's only log line mentioning service+branch was `DEBUG`-level and
   gated behind the (default-off) `business_logic_debug` toggle, so it never reached the buffer in
   normal operation; `require_endpoint_inner` had **zero** tracing calls on its success path at all.

**Fix:**
- `LogEntry` (`src/domain/models.rs`) gained `service: Option<String>` / `branch: Option<String>`.
- `LogVisitor` now also captures fields named `"service"`/`"branch"` (in `record_str` and
  `record_debug`), alongside `"message"`.
- One new `tracing::info!(service = …, branch = …, "…")` line each in `provide_spec_inner`
  (`src/application/spec_service.rs`, on a real non-dry-run, non-no-op provide, right after the
  version increments — mirrors the `#9` no-op-skip condition) and `require_endpoint_inner` (on
  successful, non-dry-run resolution). `report` paths were **not** instrumented — `#6` already
  excludes plain report viewing from the audit log as noise, and the same reasoning applies here;
  can be added later if wanted.
- `static/observability.html` shows a compact `[service/branch]` chip inline when present, omitted
  (not padded with "—") when absent — the original plan's "show — rather than empty" suggestion
  assumed a table layout; the real UI is a monospace text stream where a placeholder on every
  context-less line (the majority — auth, startup, migrations) would be noise, not signal.

**Retracted — the "bonus" timezone finding was a false alarm.** The pasted `[13:54:26]` /
`[00:54:26]` / `[02:54:26]` lines all share the identical `:54:26` minute:second, only the hour
differs — the signature of a **fixed-interval periodic task** (`tokio::time::interval` in
`src/main.rs`, confirmed), not mixed timezones. The apparent "jump" is simply the pasted excerpt
skipping intermediate ticks. No timezone bug exists; nothing to fix.

**Test:** `captures_service_and_branch_fields_from_an_event` and
`leaves_service_and_branch_none_when_absent` (`src/presentation/middleware.rs`, using
`tracing::subscriber::with_default` to exercise the real capture layer). **Scope note:** these
unit-test the capture mechanism with the exact field pattern the call sites use
(`service = &str, branch = &str`); they do **not** exercise `provide_spec_inner`/
`require_endpoint_inner` end-to-end through a live subscriber, since `tracing`'s thread-local
`with_default` subscriber isn't guaranteed to survive suspension points across tokio worker threads
in a multi-threaded async test, and a flaky test would be worse than no test here.

---

## P2 — Feature requests & polish

### 16. Dark mode has very low contrast — palette needs a real pass

**Symptom:** Dark mode is barely legible.

**Root cause:** the theme toggle (`static/js/common.js:170+`) only adds a `dark` class to `<html>`
and runs a text/logo "gimmick" (renames Sanshain→SOKA, swaps the logo). There are **no** `dark:`
Tailwind variants in the pages, Tailwind (CDN) is not configured for class-based dark mode, and
there is no dark palette CSS. So enabling dark mode does not actually restyle colors — pages keep
light backgrounds and whatever contrast falls out by accident.

**Proposed fix:** implement dark mode for real: enable class-based dark mode for the Tailwind build,
define a dark palette, and add `dark:` variants (or a dark stylesheet) across the shared
components — nav, cards (`bg-white`→dark surface), text (`text-slate-800`→light), borders, badges,
and the code/YAML viewers. Target WCAG-AA contrast. Do it component-by-component using the shared
partials so it lands consistently. Consider the `dataviz`/`artifact-design` contrast guidance for
color choices. This is a multi-PR UI effort — track sub-tasks here.

**Test:** visual review in both themes; a Playwright screenshot smoke in dark mode; a contrast
check on the primary text/background pairs.

### 2. "Show full API for a branch" view — DONE 2026-07-23

**Feature:** the UI only shows per-endpoint YAML. Add a way to view the **full assembled** spec for
a branch.

**Done:** new `services::get_full_spec` (`src/application/spec_service.rs`) reassembles a branch's
stored per-endpoint specs into one document — OpenAPI via the existing
`openapi::merge_endpoint_yamls`, AsyncAPI/proto concatenated with `\n---\n` (matching
`require_bundle`), endpoints sorted for stable output; errors `404` when no endpoint of that type
exists. Served by `GET /admin/services/{name}/branches/{branch}/full-spec?api_type=…`
(`admin_get_full_spec`, `authenticated_auth`, added to the `NOT_IN_CLIENT_CONTRACT` allowlist).
`services.html` shows a "Download full API" button on the branch view that fetches it (with the
bearer token) and downloads it as a file. Backend tested by `full_spec_merges_branch_endpoints`;
route auth + contract tests pass; frontend `node --check` clean.

### 5. OpenAPI spec with no endpoints — RESOLVED 2026-07-26 (accept, but hide from browsing)

**Decision (maintainer):** keep accepting an empty-paths provide (option (a)) — but an empty
branch, or a service whose branches are *all* empty, provides zero benefit sitting in the services
overview / "server" browsing view and must not appear there.

**Verified current behavior (unchanged):** `split_openapi` returns `Ok(vec![])` for an empty
`paths` map (`openapi.rs:83-147`, no error) — this was already true, not something added now.
`provide_spec_inner` still creates the service+branch, advances the version, and returns `202`.

**Done — visibility fix (`services.html` discovery view only):**
- New port method `list_branch_endpoint_counts()` → `(service, branch, non_deleted_endpoint_count)`
  for every branch, implemented on both backends (SQLite/PostgreSQL), delegated through
  cached/database, stubbed empty in `MockRepo`.
- `ServiceSummary.branches_endpoint_count: HashMap<branch, i64>` (additive field, like
  `branches_last_published`/`branches_expire_at`), populated in `list_services_detailed`
  (`src/application/admin_service.rs`). **`ServiceSummary.branches` itself is left unfiltered** —
  it is the raw source of truth and admin management (`admin.html`, which lists/deletes
  branches/services) needs to see and clean up empty branches too.
- `discovery.js`: new `allServiceEndpointCount` cache + `branchHasEndpoints()` /
  `serviceHasAnyEndpoints()` helpers.
- `services.html`: `renderServiceList` now only lists services with `serviceHasAnyEndpoints`;
  `showServiceBranches` filters branches to `branchHasEndpoints`. `reports.html`/`graph.html` keep
  using the unfiltered `allServiceBranches` for their branch-selector dropdowns (intentionally —
  you may want to select an empty branch there, e.g. to see what a client is missing).
- Confirmed **no graph fix was needed**: the dependency graph is built entirely from
  `dependency_graph` rows (`get_report`'s SQL joins `endpoints`), so an empty branch already
  produced zero graph nodes/edges.

**Test:** `empty_spec_branch_reports_zero_endpoint_count` — an empty-paths provide creates the
branch (still listed in unfiltered `branches`) but reports `branches_endpoint_count == 0`; a normal
provide reports the real count.

### 10. Show "last publish" per branch in the overview — DONE 2026-07-23

**Feature:** the branch cards (`static/services.html`) show only the branch name. Add the last
publish timestamp; surface it on the card.

**Done:** the overview branch cards now show "Last published <time>" (or "No publishes yet"),
sourced from the additive `ServiceSummary.branches_last_published` field (populated in
`list_services_detailed` from the `list_branch_last_published` port method), cached in
`discovery.js` and rendered in `services.html`. The `branches: Vec<String>` type was left unchanged.
Backend tested by `overview_branches_ordered_protected_then_recent` (asserts the field is
populated); frontend eslint/prettier/`node --check` clean. Note: `/admin/services` is not in the
documented `api.yaml` contract, so no schema change was needed. Historical note below is retained
for context.

**Done (the hard part — the timestamp is now trustworthy):** `branches.updated_at` previously
meant "last touched" because `ensure_branch` bumped it on read paths too. It now advances only on
a publishing change: `ensure_branch` (`sqlite_repository.rs` / `postgres_repository.rs`) no longer
bumps it, and `apply_spec_changes` does. So `updated_at` == last-published (last spec *change*; a
no-op re-provide short-circuits before `apply_spec_changes` and does not advance it — consistent
with #9). Tested by `reads_do_not_advance_last_published_but_publishes_do`. **This also fixes the
latent #12 bug** (viewing a branch used to reset its stale-cleanup timer; now only publishes do).

**TODO (display):** surface the per-branch last-published time on the overview branch cards. The
per-`(service, branch)` query already exists — the `list_branch_last_published` port method added
for #11 returns `(service, branch, updated_at)` rows. Remaining work: add an additive field
alongside `ServiceSummary.branches` (e.g. `branches_last_published: map<name, iso>` — do **not**
change the existing `branches: Vec<String>` type, which `cached_repository.rs:133` and the #11 sort
rely on), populate it in `list_services_detailed` (the map is already built there for the sort),
then `api.yaml` + the `services.html` branch-card render.

**Known gap — RESOLVED 2026-07-26 (latent finding L2):** admin manual endpoint edits went through
`update_endpoint` without bumping `updated_at`. `update_endpoint` (both backends) now advances
`branches.updated_at`, so a manual edit counts as branch activity (last-published refreshed, not
prematurely culled). Tested by `admin_edit_advances_last_published`.

**Unblocks:** #11 recency ordering (protected → newest publish → name) and #12 (TTL badge) can now
use `updated_at` correctly.

### 11. Branch ordering in the overview — DONE 2026-07-23 (incl. recency)

**Finding:** the per-service `list_branches` query is `ORDER BY b.name` (alphabetical), but the
**services-overview** branch list is built from `GROUP_CONCAT(b.name)` in `list_services_detailed`
(`src/infrastructure/sqlite_repository.rs`), which has **no** inner ordering — so the overview
branches were effectively unsorted and could vary between requests. That is the surface the tester
saw.

**Done:** `list_services_detailed` (`src/application/admin_service.rs`) now sorts each service's
branches **protected-first** (exact-match against `list_protected_branches`, e.g. master/main),
then by **most recent publish** (newest first, via the new `list_branch_last_published` port
method — see #10), then alphabetically. Tested by
`test_branches_ordered_protected_first_then_alphabetical` (mock, name tiebreak) and
`overview_branches_ordered_protected_then_recent` (sqlite, real recency).

### 12. Branch TTL in the overview — DONE 2026-07-23

**Feature:** stale branches are already cleaned up (the "Branch cleanup: deleted N stale branches"
task). Surface the remaining TTL on the branch card — at least warn when a branch will be culled in
**< 3 days**.

**Done:** `list_services_detailed` computes a per-branch `branches_expire_at`
(= last publish + `branch_max_age_days`) for **non-protected** branches only (protected branches
are exempt from `delete_stale_branches`, confirmed via its `NOT EXISTS (protected_branches …)`
guard), when cleanup is enabled (`max_age_days > 0`). `services.html` shows an amber
"Expires in Nd" / "Expiring" badge when the branch is ≤ 3 days from culling. Uses the same retention
setting the cleanup task uses, so badge and deletion agree. Backend tested by
`overview_shows_expiry_for_non_protected_branches_only`; frontend eslint/prettier/`node --check`
clean.

**Note — RESOLVED 2026-07-26 (latent finding L1):** `is_branch_protected` matched patterns
**exactly** while stale-cleanup used wildcards, so a wildcard protected pattern exempted a branch
from cleanup yet reported "not protected" everywhere else (breaking-change enforcement, version
recording, fallback, and this TTL badge). `is_branch_protected` now mirrors each backend's cleanup
matcher (SQLite `GLOB`, PostgreSQL `LIKE`) — wildcard patterns like `release/*` now protect the
branches they cover. Tested by `wildcard_protected_pattern_matches_branches`.

**Still open (separate, pre-existing):** the two backends use *different* wildcard syntaxes in
cleanup — SQLite `GLOB` (`*`, `?`) vs PostgreSQL `LIKE` (`%`, `_`) — so a pattern like `release/*`
protects on SQLite but `release/%` is needed on PostgreSQL. `is_branch_protected` now faithfully
mirrors that per-backend, but the cross-backend divergence in what a pattern *means* predates this
work and should be reconciled separately (pick one canonical wildcard syntax, translate at the
boundary).

### 15. Blame on master: attribute to the real author, not the CI

**Symptom:** On `master`, the CI is always the provider, so all master history is blamed on the CI
account/token (e.g. the `maven` token). The human who actually authored the change is lost.

**Plan (feature, DECISION NEEDED on the mechanism):** let a provide carry an *author* distinct from
the *actor* — e.g. an optional author field (git commit author / a header the CI forwards), stored
in `endpoint_version_metadata` alongside the existing `username`/`source_branch`. Fallback: if the
same change was first seen on a feature branch provided by a human, attribute master's adoption to
that human. Record both "pushed by (actor)" and "authored by (author)" and show the author as the
blame. Requires: metadata schema addition (migration), provide API/param change, and blame
rendering in `yaml.html`.

**Relevant areas:** `endpoint_version_metadata` schema (migration),
`src/application/spec_service.rs` (provide flow, blame), provide handlers + `api.yaml`,
`static/yaml.html` (blame view).

### 17. No real notion of which protected branch a feature branch descends from — DONE 2026-07-26

**Design resolved via a grilling session** (details below), specced as
[GitHub issue #1](https://github.com/paxel/sanshain-service/issues/1) (`ready-for-agent`), then
fully implemented per that spec. Summary: **`source_protected_branch`** (sticky, per-branch,
first-write-wins, admin-correctable) + **`pull_from_branch`** (per-request, `/require`-only,
non-persisting override).

**Implementation:**
- Migration: nullable `branches.source_protected_branch` (both backends).
- Port methods `set_source_protected_branch_if_unset` / `get_source_protected_branch` /
  `admin_set_source_protected_branch`, implemented on both backends plus cached/database
  delegators and a real (not stubbed) `MockRepo` tracking map.
- `ProvideRequest`/`ProvideAsyncApiRequest`/`ProvideProtoRequest`/`ProvideSpecParams` gained
  `source_protected_branch`; `RequireQuery`/`RequireEndpointParams` gained
  `source_protected_branch` and `pull_from_branch`.
- `apply_source_protected_branch_hint` (`src/application/spec_service.rs`): shared write-once +
  mismatch-logging helper, called from both `provide_spec_inner` and `require_endpoint_inner`.
  Mismatches log via `tracing::warn!(service = …, branch = …, …)`, reusing item #7's structured
  logging.
- `fallback_branch_candidates` now takes `service_id` and consults the branch's own
  `source_protected_branch` first, ahead of the service `fallback_branch` and the alphabetical
  protected-branch chain — automatically propagating to all three of its callers (`get_endpoint_yaml`,
  `get_endpoint_version_history`, `find_endpoint_with_fallback` → `require_endpoint_inner`).
- `pull_from_branch` short-circuits `require_endpoint_inner`'s loop to a single direct
  `find_endpoint` lookup, with no resolution chain and no persistence.
- New admin routes `GET`/`PUT /admin/services/{name}/branches/{branch}/source-protected-branch`
  (`admin_auth`-gated), added to the `NOT_IN_CLIENT_CONTRACT` allowlist.
- `api.yaml` updated: new `source_protected_branch` field on all three provide payload schemas;
  new shared `source_protected_branch`/`pull_from_branch` query parameters on all three require
  endpoints; the `/require` description's stale "falls back to `master` automatically" text
  (inaccurate since #13's fallback work earlier this session) corrected to describe the real
  resolution order.

**A real regression was caught and fixed during testing:** the first implementation called
`ensure_branch` unconditionally in `require_endpoint_inner` whenever a hint *could* apply, which
created a phantom branch row for a plain `/require` against a nonexistent service/branch — breaking
the pre-existing `test_require_does_not_create_phantom_service` invariant. Fixed by gating
`ensure_branch` on a hint actually being supplied (`params.source_protected_branch.is_some()`), so a
require with no hint has zero persistence side effects, matching prior behavior exactly.

**Tests** (`tests/admin_handlers_test.rs`): first-write sets the value; a differing second write is
ignored and logged as a mismatch (via a scoped `LogCaptureLayer` + `#[tokio::test(flavor =
"current_thread")]`, since a multi-threaded runtime cannot reliably keep a thread-local tracing
subscriber active across `.await` points); a matching second write is a no-op with no log; an admin
correction always overwrites and a later caller-supplied value cannot undo it; `pull_from_branch`
bypasses resolution and creates no branch row at all; a protected branch with no data still 404s
(regression guard, still correct); `get_endpoint_version_history` prefers `source_protected_branch`
over the alphabetically-earlier `main`; the admin HTTP endpoint works end-to-end through the real
router and is confirmed `403` for a non-admin. Full suite (33 groups), `cargo clippy --tests -D
warnings`, and `cargo fmt --check` all pass.

**Problem (raised by maintainer, 2026-07-26):** the service has no concept of branch ancestry.
When resolving a fallback for a branch that lacks its own data, the only signals available are:
1. A single, **admin-configured, per-service** `fallback_branch` setting (`set_fallback_branch`) —
   static, does not vary per feature branch, must be set manually.
2. Otherwise, **every** protected branch, tried in **alphabetical order**
   (`list_protected_branches`: `ORDER BY pattern`) — e.g. with `main`, `release/1.5`, `release/1.6`
   all protected, a feature branch actually cut from `release/1.6` would try `main` first, then
   `release/1.5`, before ever reaching `release/1.6` — an arbitrary pick with no relation to which
   release the branch was actually created from.

There is currently no field anywhere in the `/provide*` **or** `/require*` payloads
(`ProvideRequest`, `RequireQuery`/`RequireEndpointParams`, `src/presentation/handlers/api.rs`) that
identifies a branch's parent/base — Sanshain only ever receives an isolated branch name string on
each call, never git ancestry.

**This is not just a `/provide`-side concern.** `find_endpoint_with_fallback` (which
`fallback_branch_candidates` feeds into) has three call sites, all sharing the exact same
ambiguity: `get_endpoint_yaml` (`spec_service.rs:708`, the current-spec viewer), the version-history
fallback added for #3/#4/#13 (`get_endpoint_version_history`, `:823`), and — most consequentially —
**`require_endpoint_inner`** (`:900`), which is what actually decides what spec content a *client*
gets served when it calls `/require` for a branch the service hasn't published. A wrong fallback
pick there isn't just a confusing history view; it's a client building or testing against the wrong
release's contract, silently.

**Impact:** the fallback/inherited-history behavior added for #3/#4 and #13, and the pre-existing
`/require` fallback, are only as good as "some protected branch happens to have this endpoint" —
with multiple protected branches (a common setup: `main` plus several `release/X.Y` lines), the
branch picked can be the wrong one, silently showing history, a fallback spec, or served client
content from an unrelated release.

**An added wrinkle for the eventual decision:** the hint could originate from either side. A
*service* provide could declare what its own branch descends from; a *client* require could equally
declare what its branch descends from. Since Sanshain correlates branches across repos purely by
name convention (a client's "feature-x" and a service's "feature-x" are unrelated git branches in
unrelated repos that happen to share a name), these two declarations are two independent opinions
about the same conceptual ancestry and could disagree — e.g. a client believes it's based on
`release/1.6` while the service that owns the branch name believes `release/1.5`. Whichever design
is chosen needs to say which side's hint wins, or whether both are recorded and reconciled somehow.

**Is this solvable without git access?** Correctly and generally — no. Sanshain has no git
integration and only sees whatever a `/provide` or `/require` call tells it; there is no way to
derive "this branch was cut from that commit on that branch" from the API surface as it exists
today.

**A partial, git-access-free mitigation that *is* feasible:** the entity provisioning a branch
already knows its own git parent — that information just isn't being forwarded to Sanshain. An
optional `base_branch` hint (e.g. `{"base_branch": "release/1.6"}`, best-effort) on **both**
`/provide*` and `/require*` would let `fallback_branch_candidates`/`find_endpoint_with_fallback` try
the *declared* parent first, before falling back to the current alphabetical-protected-branch
behavior. This does not require git access on **Sanshain's** side — only a small, additive protocol
change (`ProvideRequest` and `RequireQuery`/`RequireEndpointParams` fields, `api.yaml`, stored e.g.
in `endpoint_version_metadata` or a new `branches.base_branch` column).

Critically, this doesn't have to mean "every CI author manually adds a flag to their pipeline YAML"
— it's more realistic than that. **Client integrations that run inside the checkout** (the Sanshain
Maven plugin is the concrete example here — it would forward the hint on the `/require` calls it
makes, not `/provide`; any future client library fits the same shape) sit exactly where git *is*
available, unlike Sanshain's server. Such a plugin could auto-detect the base branch itself — via a
local `git merge-base`/`git symbolic-ref` against the checkout, or by reading whichever CI
platform's own env var already carries it (GitHub Actions' `GITHUB_BASE_REF`, GitLab's
`CI_MERGE_REQUEST_TARGET_BRANCH_NAME`, etc.) — and forward it automatically, with zero manual wiring
from whoever writes the pipeline config. Service-side CI (calling `/provide`) could do the same for
its own branch. (The Maven plugin is a **separate repository** (`sanshain-maven-plugin`), not part
of this codebase — any change there is out of scope for this repo's backlog, but the *protocol*
side, the optional `base_branch` field this server would need to accept on both `/provide` and
`/require`, belongs here.)

Either way — human-supplied or plugin-auto-detected — it remains a *hint*, not authoritative
ancestry: nothing stops a caller from omitting it or getting it wrong, and it wouldn't retroactively
fix branches created before the field existed.

**Recommendation:** don't implement yet — this is a protocol/product decision (is CI/plugin
cooperation realistic here? is "hint, not truth" an acceptable model? which side's hint wins if
provide and require disagree, or are both stored and reconciled? does the ordering need to be
configurable beyond a single per-service `fallback_branch`?). Get the call before touching
`ProvideRequest`/`RequireQuery`/`api.yaml`.

**Relevant areas:** `src/presentation/handlers/api.rs` (`ProvideRequest`, `RequireQuery` and
siblings), `src/application/spec_service.rs` (`fallback_branch_candidates`,
`find_endpoint_with_fallback`, `require_endpoint_inner`),
`src/infrastructure/*_repository.rs` (`list_protected_branches`, `set_fallback_branch`),
`api.yaml`, `src/infrastructure/migrations/` (if a stored `base_branch` is chosen over metadata-only).

---

**Design session 2026-07-26 (in progress — not yet decided, nothing implemented).** Grilled through
concrete scenarios (server/client branching off master vs. off a release line, merge-order races,
and — the decisive one — a hotfix cut locally off `release/1.0` while `master` has since deprecated
and removed endpoints `release/1.0` still needs). Findings:

- Confirmed in code: requiring against a branch that **is itself protected** but has no data already
  fails correctly today (`find_endpoint_with_fallback` returns `None` without ever substituting
  another protected branch) — the "must never silently fall back to master from a *different*
  protected branch" requirement is **already met** for that specific case. No change needed there.
- The real gap is a *local development machine, before any PR/CI context exists* — not a rare edge
  case, the **default state** of any branch before it has a pull/merge request. CI env vars
  (`GITHUB_BASE_REF` etc., see the CI survey above) only populate once a PR exists, so they cannot be
  the whole answer.
- `git merge-base` computed **locally** (a normal dev clone has full history, unlike CI's typical
  shallow clone) against the **known protected branches** correctly identifies the true divergence
  point even pre-PR. This requires knowing the candidate set first — confirmed
  `GET /branches/protected` **already exists** (`api_auth`-gated, `src/lib.rs:76`) and already serves
  exactly this list, so no new endpoint is needed for a client to fetch candidates.

**Emerging design (names confirmed 2026-07-26; not yet implemented — still awaiting go-ahead):**
- **`source_protected_branch`** — a new **per-branch** sticky setting (deliberately not named
  `source_branch`, which already exists at a different grain: `EndpointVersion.source_branch` is
  recorded per *version*, for blame; this is per *branch*) holding which protected branch should be
  treated as authoritative when this branch has no data of its own.
- Unset → today's last resort (service `fallback_branch` / alphabetical) applies unchanged.
- A caller (client `/require` or service `/provide`) may supply a value; **first write wins** — once
  set (by that first write or by an admin), later caller-supplied values for the same branch are
  **ignored**, specifically to prevent flip-flopping between different computed values across
  different machines/CI runs. Only an admin can change it after that point.
- Work required elsewhere, out of scope for this repo: the Maven plugin (separate repository,
  `sanshain-maven-plugin`) would need to actually implement the local `merge-base` computation
  against `/branches/protected` and start sending the value.

**Client/service disagreement — resolved with a second, separate mechanism: `pull_from_branch`.**
The client and the service are **independent repositories**, correlated only by matching branch
*names* (established earlier in this doc). Each independently computes its own
`merge-base`-derived answer against its *own* git history — for a branch name that exists in both,
the two sides can genuinely disagree about which protected branch it actually diverged from. This
isn't a bug, it's a structural consequence of correlating two unrelated commit graphs by name alone.

Resolution: **`pull_from_branch`** — a **per-request** parameter on `/require` only (not `/provide`).
Unlike `source_protected_branch`, it does **not** persist or overwrite any stored value — it is a
one-shot override for that single call: when supplied, it bypasses `source_protected_branch` and all
other fallback resolution entirely and fetches data from exactly the named branch. This is the
client's escape hatch when it disagrees with (or can't rely on) whatever `source_protected_branch`
currently holds, **without** corrupting the shared, persisted state that other callers depend on —
it only affects the one call that used it. This addresses the earlier discomfort with a destructive
"client wins outright" global override: the override is now scoped and non-destructive rather than a
silent, permanent discarding of the service's own answer.

---

### 18. The whole observability/audit view is open to any authenticated user, not just admins — PARTIALLY RESOLVED 2026-07-26 (audit log restricted; stats/raw logs deliberately left open)

**Problem (raised by maintainer, 2026-07-26, surfaced while fixing #8):** every **read** route under
`/admin/observability/*` — `stats`, `logs` (the raw tracing stream), `audit-logs`, and
`audit-logs/export` (CSV) — was gated by `authenticated_auth` (any valid logged-in session), not
`admin_auth` (`src/lib.rs:123-128`). Only the one **write** route, `debug-config-update` (toggling
debug logging on/off), required `admin_auth`. Despite living under an `/admin/` path, any
registered, non-admin user could view the full audit trail (every `PROVIDE_SPEC`, `REQUIRE_*`,
`CREATE_TOKEN`/`REVOKE_TOKEN`, branch/service deletions, settings changes, across all users).

**Decision (maintainer):** restrict the **audit log** specifically — `audit-logs` and
`audit-logs/export` now require `admin_auth` (`src/lib.rs`). `stats` and `logs` (the raw tracing
stream) were **deliberately left** on `authenticated_auth` — narrower scope than my original framing
of "the whole observability view," matching exactly what was asked ("change the audit to admin
only"), not the broader page.

**Done:**
- `src/lib.rs`: `audit-logs` and `audit-logs/export` routes switched from `authenticated_auth` to
  `admin_auth`.
- `static/observability.html`: `loadAuditLogs()` now shows an explicit "Admin access required to
  view the audit log." message in the table on a `403`, instead of silently leaving it blank
  forever for a non-admin.
- Test `audit_logs_are_admin_only`: registers a real non-admin user (via the actual
  register/approve/login flow), asserts `403` on both audit routes, then asserts the seeded admin
  still gets `200` on both.

**Still open — not decided, not touched:** whether `stats` and `logs` (the raw tracing stream, which
can carry internal error detail/stack traces/DB errors) should also become admin-only. Left as a
separate, smaller open question rather than assumed.

**Relevant areas:** `src/lib.rs` (route middleware), `static/observability.html`
(`loadAuditLogs`, `checkSession`, `applyAdminState`).

---

## Cross-cutting notes

- Several items touch the same files (`yaml.html`, `services.html`/`clients.html`, the provide and
  report handlers). Sequence them so shared helpers (e.g. a single `yaml.html` link builder, a
  per-branch last-activity query) are introduced once and reused.
- #10, #11, #12 all want a **per-branch last-activity / expiry query** — build that once.
- #3/#4, #13, #14 all revolve around the branch/history model — settle the #3/#4 decision first, as
  it shapes the others.
- #17 is the underlying limitation of the fallback logic #3/#4 and #13 both landed on
  (`fallback_branch_candidates`): it picks *a* protected branch, not necessarily the *right* one
  when several exist. Not blocking — #3/#4/#13 are still correct improvements over 404/empty — but
  worth resolving before leaning on the fallback further.
