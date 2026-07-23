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

### 1. Favorites disappear "after update"

**Symptom:** After an update, a user's starred favorites (services/clients) are gone.

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

### 13. Client-only branch → clicking an endpoint fails with `Failed to load versions: 404`

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

### 14. Browsing a client's branch, the "→ server" link shows the same page

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

### 3 & 4. Feature branches always show "No history" — even a branch cut from master with no changes

**Symptom:** Viewing an endpoint on a feature branch shows no version history. A branch created
from `master` with identical content also shows "no history", which is confusing because `master`
has history.

**Root cause:** version rows (`endpoint_versions`) are only written when the branch is protected
(`if is_protected { … }` in `apply_spec_changes`, `src/infrastructure/sqlite_repository.rs:1996`
and `postgres_repository.rs:1933`). Feature branches never accumulate history.

**Reproduce first — the symptom is ambiguous.** The 1.5.1 current-spec fallback
(`loadCurrentSpecFallback`) **is confirmed present in the tested build** (HEAD `06a20ce`), so
"tested an old build" is ruled out. When that fallback fires it shows the endpoint's **current
spec**, *not* the "No version history found" text — so seeing the empty message means the fallback
did **not** fire. Determine which reality holds before choosing a fix:
- **(b) Endpoint not materialized on the branch** → `get_endpoint_id` returns `None` →
  `/admin/endpoint-versions` returns `404`, the fallback also 404s → this is the **same failure as
  #13**, not an empty-history state. Check whether creating a branch from master actually copies
  endpoints onto the new branch, or whether the endpoint list shows rows that don't exist server-side.
- **(c) Fallback bug** — the endpoint exists (200 `[]`) but the fallback fetch fails or is skipped.
  Check the `res.length === 0` path and the `/admin/endpoints/yaml` call in `yaml.html`.
- **(d) Label perception** — the fallback *is* showing the current spec as a single "Version 1"
  card and the tester reads that as "no history".

**Fix depends on the reality:**
- (b) → fix under #13 (branch/endpoint existence + 404 handling); possibly ensure branching copies
  endpoints, or make the UI not list unprovided endpoints as clickable.
- (c) → fix the fallback.
- (d) → **(C)** relabel the single card, e.g. "Current spec — no branch history yet (version
  tracking begins on protected branches)". Nearly free.

**Only if it's (d) and the product wants real feature-branch history**, consider the heavier option
**(A) inherited view**: when a feature branch has no history, show the base/protected branch's
history for the same endpoint (read-time fallback, no schema change). Do **not** default to (A);
it is the biggest option and is unjustified unless (b)/(c) are excluded and (C) is deemed
insufficient. (Option (B) — record versions on all branches — is explicitly out of scope: more
storage, and it abandons the "protected = versioning gate" design.)

**Relevant areas:** `src/application/spec_service.rs` (`get_endpoint_version_history`),
`src/infrastructure/*_repository.rs`, `static/yaml.html`, branch-creation/copy logic (for (b)).

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

### 8. Token create/revoke in the audit log — keep, drop, or separate? **DECISION NEEDED**

**Symptom:** The audit timeline shows `CREATE_TOKEN` / `REVOKE_TOKEN` rows
(`src/presentation/handlers/auth.rs:239,265`), which the tester questioned.

**Discussion:** token lifecycle is genuinely security-relevant and arguably *should* be audited —
but it is noise in a timeline meant for spec/branch activity. Decide: (a) keep as-is; (b) move
token events to a separate security/admin audit view and out of the spec timeline; (c) drop.
Recommendation: **(b)**. No code until the call is made.

### 7. Observability log entries always missing `service` and `branch`

**Symptom:** The observability log viewer shows service/branch columns that are always empty; log
lines are plain `sanshain_service: …` messages.

**Root cause (to confirm):** the in-memory/tracing log events do not carry structured `service` /
`branch` fields (`src/infrastructure/telemetry.rs` sets up tracing but events are emitted as bare
messages). The viewer has the columns but nothing populates them.

**Proposed fix:** attach `service` and `branch` as structured fields on the relevant tracing events
(provide/require/report paths) via `tracing::info!(service = …, branch = …, "…")`, and surface
those fields in the log-buffer entry the observability page reads. Where a log line has no
service/branch context, show "—" rather than an empty cell.

**Bonus (spotted in the pasted logs):** the "Branch cleanup: deleted N stale branches" lines print
with inconsistent timezones (`[13:54:26]` vs `[00:54:26]` / `[02:54:26]` for events emitted moments
apart). Looks like a local-vs-UTC mix in the log timestamp formatter — worth a look while in
`telemetry.rs`.

**Test:** unit/integration test asserting a provide emits a log/observability entry carrying the
correct `service` and `branch`.

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

### 2. "Show full API for a branch" view

**Feature:** the UI only shows per-endpoint YAML. Add a way to view the **full assembled** spec
(OpenAPI/AsyncAPI/proto) for a branch — the merged document across all its endpoints. Likely a new
read endpoint that concatenates/serves the stored per-endpoint specs into one document, plus a
"View full API" button on the branch view with copy/download (reuse the existing viewer chrome).

**Relevant areas:** `src/application/spec_service.rs` (assemble full spec), a new handler in
`src/presentation/handlers/api.rs`, `static/services.html` (button) + a viewer, `api.yaml`.

### 5. OpenAPI spec with no endpoints — define the behavior **DECISION NEEDED**

**Symptom/question:** what should happen when a provided OpenAPI document has no paths/operations?

**Plan:** decide and then enforce one behavior: (a) accept and store an "empty" branch, and have the
UI show a clear "No endpoints in this spec" state (not a broken/blank view); or (b) reject the
provide with a `400` explaining that at least one operation is required. Check current behavior in
`parse_spec_endpoints` (`src/application/spec_service.rs`) first, then make it intentional and
tested. Recommendation: **accept + explicit empty state**, since a service may legitimately start
empty.

### 10. Show "last publish" per branch in the overview — DATA FIXED 2026-07-23, display TODO

**Feature:** the branch cards (`static/services.html`) show only the branch name. Add the last
publish timestamp; surface it on the card.

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

**Known gap:** admin manual endpoint edits (`update_endpoint` / `update_endpoint_manual`) are a
write path that does **not** currently bump `updated_at`. Decide whether an admin edit should count
as branch activity; if so, add the same bump there.

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

### 12. Branch TTL in the overview

**Feature:** stale branches are already cleaned up (the "Branch cleanup: deleted N stale branches"
task). Surface the remaining TTL on the branch card — at least warn when a branch will be culled in
**< 3 days** (e.g. an amber "expires in 2d" badge). Derive from the same cutoff the cleanup task
uses so the badge and the actual deletion agree.

**Relevant areas:** the stale-branch cutoff logic in `src/application/` / cleanup task, a
per-branch last-activity query, `static/services.html`.

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

---

## Cross-cutting notes

- Several items touch the same files (`yaml.html`, `services.html`/`clients.html`, the provide and
  report handlers). Sequence them so shared helpers (e.g. a single `yaml.html` link builder, a
  per-branch last-activity query) are introduced once and reused.
- #10, #11, #12 all want a **per-branch last-activity / expiry query** — build that once.
- #3/#4, #13, #14 all revolve around the branch/history model — settle the #3/#4 decision first, as
  it shapes the others.
