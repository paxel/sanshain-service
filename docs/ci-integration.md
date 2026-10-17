# CI Integration

This guide covers how to integrate Sanshain into your CI/CD pipelines for automated contract validation and dependency management.

> **Prerequisites:** A running Sanshain instance with authentication configured. See [Getting Started](getting-started.md) for installation and [Administration](administration.md) for configuration.

## API Tokens

CI pipelines should use **API tokens** instead of username/password credentials. Tokens are long-lived, revocable, and scoped to a single user.

### Creating a Token

1. Log in to `/account.html` in your browser.
2. In the **API Tokens** section, enter a name (e.g. `jenkins-ci`) and an expiry (e.g. 365 days).
3. Click **Create Token**. The raw token (prefixed `san_`) is shown **once** — copy it immediately.

Or via the API:

```bash
TOKEN=$(curl -s -X POST http://localhost:3000/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"ci-user","password":"..."}' | jq -r .token)

curl -s -X POST http://localhost:3000/auth/tokens \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"jenkins-ci","expires_in_days":365}'
```

### Using a Token

Pass the token as a Bearer header in all API calls:

```bash
curl -H "Authorization: Bearer san_xxxxxxxxxxxx" \
  http://localhost:3000/provide ...
```

Store the token in your CI system's secret management (e.g. GitHub Actions secrets, Jenkins credentials, GitLab CI variables).

## Stability from the Pipeline

Every Provide declares a **stability** — and the pipeline is the natural place to decide it:

- **Release-branch builds** (`main`, `master`, release branches) provide with `"stability": "ga"` — the version becomes immutable.
- **Feature-branch builds** provide with `"stability": "snapshot"` — overwritable work-in-progress that expires when unused.

Every build provides as `snapshot` unless the pipeline sets the ga switch (`SANSHAIN_GA=true`, `-Dsanshain.ga=true`, or `--ga` — see [How stability is decided](sanshain-yaml.md#how-stability-is-decided)); raw `curl` pipelines set the field themselves. The **version** is never a pipeline concern — it is read from the spec file (`info.version`, or the `// sanshain-version:` comment for proto).

## Streams from the Pipeline

Alongside stability, the pipeline declares the **stream** ([User Guide — Declaring the Stream](user-guide.md#declaring-the-stream)):

- **Trunk CI** (the build of your default branch) adds `trunk: true` to provides and `trunk=true` to requires — this maintains the main graph. Nightly re-runs of an unchanged trunk build are wanted: they keep trunk data fresh instead of aging out.
- **Release pipelines and hotfix builds** add `tag: "<sanshain-branch>"` — the calls update that release's graph instead of trunk. The branch must already exist (created by a releaser at cut time). An unknown tag — or `trunk` and `tag` together — does not fail the call: the build is recorded in the dev graph only and the response carries a `Sanshain-Warning` header (see below).

Streams never change what is served or any version rule — they only decide which graph records the call.

### The catalog push

A build with `catalogInfo` in its `sanshain.yaml` also pushes the project's Backstage
`catalog-info.yaml` (`PUT /catalog/{systemname}`, once per build, `trunk` as above). Sanshain reads
the `Component` named like the project and of it: `spec.dependsOn` (the Peripheral Resources drawn
in the graph), `spec.owner`, `metadata.description`, the `backstage.io/source-location` annotation
and `metadata.links` — the owner, description, repository and link chips shown on the system's node
and page. Nothing else is read; see [sanshain.yaml — Catalog Phase](sanshain-yaml.md#catalog-phase)
for the exact rules. Nightly re-runs of an unchanged file change nothing.

### The `Sanshain-Warning` header

The graph is informational and never fails a build. When Sanshain cannot record a call the way it was
declared, the call still succeeds and the response carries one `Sanshain-Warning` header line per
problem — on provides, requires, bundles and catalog pushes, dry-runs and `304` answers included:

- `no sanshain-branch '<name>' - this build was not recorded in a release graph`
- `trunk and tag are mutually exclusive - this build was recorded in neither`
- the reason a catalog push recorded nothing (`recorded: false`)

The value is plain ASCII: any other character — from a tag name, say — is escaped as `\u{XXXX}`
(the Unicode code point in hex, at least four digits), so `Größe` arrives as `Gr\u{00F6}\u{00DF}e`.

The clients print each warning. Turn on **strict mode** to fail the build on any of them instead;
raw `curl` pipelines can check the header themselves.

## Dry-Run Mode (PR Validation)

All three core endpoints (`/provide`, `/require`, `/require-bundle`) support a **`dry_run`** parameter. When set to `true`, the request runs full validation but **does not persist any data**:

- `/provide` with `dry_run: true` — parses the spec, reads and validates the version, splits endpoints, and applies the version rules (GA immutability, semver honesty), but stores nothing.
- `/require` with `dry_run=true` — resolves the endpoint at the pinned version, but does not record a dependency.
- `/require-bundle` with `dry_run: true` — resolves all requested endpoints, but does not record any dependencies.

This is designed for **PR validation pipelines**: catch a forgotten version bump or a semver lie before the merge, without polluting the database with temporary data.

### Example: Validate a Provider Spec

```bash
# In the service's CI pipeline (e.g. on PR)
curl -f -X POST http://localhost:3000/provide \
  -H "Authorization: Bearer $SANSHAIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"producername\": \"UserService\",
    \"stability\": \"ga\",
    \"openapi_yaml\": $(cat openapi.yaml | jq -Rs .),
    \"dry_run\": true
  }"
```

If the Provide would be rejected by the version rules — the `info.version` is already GA with different content, or the changes are breaking without a major bump — the request returns `409 Conflict` with a `proposed_version`: the CI job fails and the developer knows exactly which version to set before merging.

### Example: Validate Client Dependencies

```bash
# In the client's CI pipeline (e.g. on PR)
curl -f "http://localhost:3000/require?consumername=WebApp&producername=UserService&version=2.1.0&path=/users&method=GET&dry_run=true" \
  -H "Authorization: Bearer $SANSHAIN_TOKEN"
```

If the pinned version does not exist, the response returns `404` (Unknown — a Pin configuration error); if the version exists but lacks the endpoint, `410` (Absent — deliberately not part of that version's API).

### Example: Validate a Bundle

```bash
curl -f -X POST http://localhost:3000/require-bundle \
  -H "Authorization: Bearer $SANSHAIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "consumername": "WebApp",
    "producername": "UserService",
    "version": "2.1.0",
    "endpoints": [
      {"path": "/users", "method": "GET"},
      {"path": "/users/{id}", "method": "DELETE"}
    ],
    "dry_run": true
  }'
```

If some endpoints are missing from the pinned version, the response returns `410` naming the missing endpoints.

## Idempotency & Caching

Sanshain is designed so pipelines can provide and require unconditionally on every build.

### 1. Content-Based No-Op

Re-providing byte-identical content is a no-op regardless of stability or caller — the `changes` summary comes back all zero and nothing is stored. CI re-runs of the same commit never fight, and unconditional "provide on every build" causes no churn.

The service normalizes every upload before comparing, hashing and storing it: CRLF and lone CR
line endings become LF, and a leading UTF-8 BOM is removed. The same spec therefore hashes the
same from every platform. The comparison is **byte-for-byte on the normalized text**: whitespace
and comments still count as content. A `.gitattributes` rule is still good practice, so the
files in your repository match what Sanshain stores:

```gitattributes
*.yaml text eol=lf
*.proto text eol=lf
```

### 2. Version Conflicts Are Self-Service

There is no pull-merge-retry loop. If a Provide is rejected `409`, the body carries `proposed_version` — the next free number, bumped by what actually changed. The fix is always local: set the spec file's version to the proposal and republish. Two developers editing the same spec coordinate in git, as with any other file; the next publish after their merge converges the snapshot.

### 3. ETag Caching on Requires

All require endpoints return an `ETag`; sending it back via `If-None-Match` yields `304 Not Modified` when the content is unchanged, letting pipelines skip redundant code generation. A GA Pin can never change content; a snapshot Pin can — the ETag detects exactly that.

## Typical CI Pipeline

### Provider Pipeline (service that publishes an API)

```
PR opened → dry_run provide as ga (validate version rules) → merge → provide as ga (store)
feature branch push → provide as snapshot (iterate freely)
```

1. **On PR**: Run `/provide` with `dry_run: true` and `"stability": "ga"` to validate the spec and its version against the line. If the version needs a bump, the PR is blocked with the proposal.
2. **On merge to a release branch**: Run `/provide` with `"stability": "ga"` to store the immutable version.
3. **On feature branches** (optional): Provide with `"stability": "snapshot"` so opted-in consumers can pin work-in-progress.

### Consumer Pipeline (client that depends on an API)

```
PR opened → dry_run require (validate pins) → merge → require (record + generate)
```

1. **On PR**: Run `/require` or `/require-bundle` with `dry_run: true` to verify all pinned versions exist and contain the required endpoints. If not, the PR is blocked with a descriptive error.
2. **On merge to main**: Run `/require` or `/require-bundle` without `dry_run` to record the dependency and generate client code from the returned YAML snippet.

## Descriptive Error Messages

All error responses include actionable information in the response body:

| Status                   | Meaning                                                                                        |
|--------------------------|------------------------------------------------------------------------------------------------|
| `409 Conflict`           | Rejected by the version rules; `proposed_version` names the next free version to publish as.   |
| `404 Not Found`          | Unknown — the Producer or the pinned version does not exist. Fix the Pin.                      |
| `410 Gone`               | Absent — the pinned version exists and deliberately lacks the endpoint(s); bundles name them.  |

These messages are designed to be shown directly in CI logs so developers can quickly identify and fix contract issues.

## GitHub Actions Example

Use `vars.SANSHAIN_URL` for the URL and `secrets.SANSHAIN_TOKEN` for the token. This allows you to update the service URL for all projects by changing a single Organization or Repository variable.

```yaml
name: Contract Validation
on: pull_request

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Validate API contract (dry run)
        run: |
          curl -f -X POST ${{ vars.SANSHAIN_URL }}/provide \
            -H "Authorization: Bearer ${{ secrets.SANSHAIN_TOKEN }}" \
            -H "Content-Type: application/json" \
            -d "{
              \"producername\": \"my-service\",
              \"stability\": \"ga\",
              \"openapi_yaml\": $(cat openapi.yaml | jq -Rs .),
              \"dry_run\": true
            }"
```

## Maven / Gradle Integration

Store the API token and URL in your build tool's credentials or properties store:

**Maven `settings.xml`:**
```xml
<profiles>
  <profile>
    <id>sanshain-config</id>
    <properties>
      <sanshain.url>https://sanshain.corp.com</sanshain.url>
    </properties>
  </profile>
</profiles>

<servers>
  <server>
    <id>sanshain</id>
    <username>ignored</username>
    <password>san_xxxxxxxxxxxx</password>
  </server>
</servers>
```

**Gradle `gradle.properties`:**
```properties
sanshain.url=https://sanshain.corp.com
sanshainToken=san_xxxxxxxxxxxx
```

> **Note:** Dedicated build tool plugins are under development. Check the [Sanshain GitHub organisation](https://github.com/paxel) for updates.

## Webhooks

Sanshain emits events over generic webhooks: a signed `POST` to a URL you register, for every
fact the registry records and every Provide it rejects. A Producer's events go to the hooks
registered on that Producer, to the hooks of every Consumer that currently requires it (in the
dev or trunk graph), and to the global hooks an administrator registers. Hooks are managed on
the admin page (global), and on the Producer and Consumer pages (a system's own hooks) — see
[Administration — Webhooks](administration.md#webhooks). This section is what a receiver has
to know.

### Event catalogue

Every event is a graph fact or a rejection; there are no admin or audit events.

| Type                  | When                                                                                                                                                                      | `data` fields                                                                                                                                                                                                                                                                                            |
|-----------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `version.published`   | A Provide succeeded (a new Version, an overwritten Snapshot, or a Promotion). The idempotent no-op re-provide of identical content emits nothing.                          | `producer`, `api_type` (`openapi`/`asyncapi`/`proto`), `version`, `stability` (`snapshot`/`ga`), `promoted` (bool), `stream` (`dev`, `trunk` or a Sanshain-branch name), `actor` (username or `null`), `compatibility` (`null`, or `{"against": "<previous GA>", "impact": "none|patch|minor|major"}` on a GA Provide with a previous GA) |
| `provide.rejected`    | A Provide was refused by a version rule.                                                                                                                                  | `producer`, `api_type`, `version` (the attempted one), `rule` (`immutable_ga`, `snapshot_on_ga`, `breaking_without_major`, `ga_requires_releaser`), `message`, `proposed_version` (the next free Version, or `null`), `actor`                                                                                |
| `version.deprecated`  | A Provide marked an element deprecated that was not deprecated in the previous GA (every deprecated element when there is no previous GA).                                | `producer`, `api_type`, `version`, `stability`, `elements` (list of `{"path", "method"}`), `document_deprecated` (true when every element of the document is deprecated)                                                                                                                                  |
| `dependency.recorded` | A require recorded an edge the Consumer did not hold yet — `(Consumer, Producer, endpoint, pinned Version, stream)`. Re-recording an existing edge emits nothing.          | `consumer`, `producer`, `api_type`, `version`, `stability`, `snapshot_pinned` (bool), `stream`, `endpoints` (the endpoints new to the edge, `{"path", "method"}`)                                                                                                                                         |
| `branch.cut`          | A Sanshain-branch was created. Global hooks only — a cut is about no Producer.                                                                                            | `name`, `source` (`trunk` or the source branch), `as_of`, `cut_at`, `actor`                                                                                                                                                                                                                              |
| `catalog.recorded`    | A catalog push ran, whether or not Sanshain could read the file. Delivered to the system's hooks (as a Producer and as a Consumer) and to global hooks.                    | `system`, `trunk` (bool), `recorded` (bool), `warning` (the reason when `recorded` is false, else `null`), `resources` (the number of Peripheral Resources recorded)                                                                                                                                       |
| `webhook.test`        | The hook's "Send test" button. Sent to that hook alone; cannot be subscribed to.                                                                                          | `hook_id`, `message`                                                                                                                                                                                                                                                                                     |

Dry-runs emit nothing. A global hook may be narrowed to an event-type set and to a
Producer-name glob (`*` and `?`); a glob admits only events about a matching Producer, so a
hook narrowed to a domain never receives `branch.cut`.

### The envelope

Every delivery carries the same JSON envelope; the `json` format sends it as the body:

```json
{
  "id": "6f1c0c7e-8d3a-4b86-9e8e-2f3c0a1d5b77",
  "type": "version.published",
  "occurred_at": "2026-10-17T09:12:44Z",
  "sanshain_version": "2.6.0",
  "data": {
    "producer": "orders",
    "api_type": "openapi",
    "version": "2.0.0",
    "stability": "ga",
    "promoted": false,
    "stream": "trunk",
    "actor": "alice",
    "compatibility": { "against": "1.4.0", "impact": "major" }
  },
  "links": {
    "producer": "https://sanshain.corp/producers.html?service=orders",
    "version": "https://sanshain.corp/producers.html?service=orders&api_type=openapi&version=2.0.0",
    "graph": "https://sanshain.corp/graph.html"
  }
}
```

- `id` is a UUID, unique per event; a redelivery carries the same `id`, so receivers can
  de-duplicate on it.
- `occurred_at` is RFC 3339, UTC, second precision.
- `links` holds deep links into the UI and is **omitted** when Sanshain does not know its public
  base URL (`PUBLIC_BASE_URL`, or the origin of the OIDC redirect URL — see
  [Configuration](configuration.md)). Per type: `producer`, `version` and `graph` for
  `version.published` and `version.deprecated`; `producer` and `audit` for `provide.rejected`;
  `consumer`, `producer`, `version` and `graph` for `dependency.recorded`; `graph` for
  `branch.cut`; `producer`, `consumer` and `graph` for `catalog.recorded`; `admin` for
  `webhook.test`.

Three headers accompany every delivery, in every format:

| Header                | Value                                                                       |
|-----------------------|-----------------------------------------------------------------------------|
| `Sanshain-Event`      | The event type (`version.published`, …).                                    |
| `X-Sanshain-Delivery` | The delivery id — one per hook per event; a redelivery reuses it.           |
| `Sanshain-Signature`  | `t=<unix seconds>,v1=<hex HMAC-SHA256 of "<t>.<body>">` under the hook's secret. |

The `Content-Type` is `application/json` and the `User-Agent` is `Sanshain/<version>`. There are
no custom headers and no bearer tokens: the signature is the authentication.

### Verifying the signature

The secret (`whsec_` + 64 hex characters) is shown once when the hook is created or its secret
is rotated. Compute HMAC-SHA256 over the timestamp, a dot and the **raw** request body, compare
in constant time, and reject timestamps older than five minutes to defeat replays:

```js
// Node.js receiver (Express with the raw body kept)
const crypto = require("node:crypto");

function verify(req, secret) {
  const header = req.get("Sanshain-Signature") || "";
  const parts = Object.fromEntries(header.split(",").map((p) => p.split("=")));
  const t = Number(parts.t);
  if (!t || Math.abs(Date.now() / 1000 - t) > 300) return false; // older than 5 minutes
  const expected = crypto
    .createHmac("sha256", secret)
    .update(`${t}.${req.rawBody}`)
    .digest("hex");
  const given = Buffer.from(parts.v1 || "", "hex");
  return given.length === 32 && crypto.timingSafeEqual(given, Buffer.from(expected, "hex"));
}
```

The same in Python:

```python
import hmac, hashlib, time

def verify(headers, raw_body: bytes, secret: str) -> bool:
    parts = dict(p.split("=", 1) for p in headers["Sanshain-Signature"].split(","))
    t = int(parts["t"])
    if abs(time.time() - t) > 300:
        return False
    expected = hmac.new(secret.encode(), f"{t}.".encode() + raw_body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, parts.get("v1", ""))
```

Sign the body bytes exactly as received — do not re-serialize the JSON first.

### Formats

A hook's `format` picks the body shape; the headers and the signature are the same for all three.
Formats are body shapes, not connectors: every one is a signed `POST` to the URL you give.

- `json` — the envelope above.
- `slack` — `{"text": "..."}`, the shape a Slack incoming webhook accepts, with the fact in one
  line of mrkdwn and, when links are known, an `<url|Open in Sanshain>` link.
- `teams` — the Adaptive Card message a Teams incoming webhook (Workflows) accepts:
  `{"type": "message", "attachments": [{"contentType": "application/vnd.microsoft.card.adaptive",
  "content": {...}}]}`, with the event type as the title, the fact as text and, when links are
  known, an `Action.OpenUrl` button.

### Retries, the delivery log and the allow-list

- A `2xx` answer is a delivery; the body of the answer is never read. Anything else — another
  status, a connection error, the 10 s timeout — is a failed attempt.
- Failed attempts are retried with exponential backoff: 30 s after the first failure, doubling
  up to one hour between attempts, **until 24 hours after the event occurred**; then the delivery
  is `dead`. The window is measured from the event, not from the first attempt, so a hook paused
  for a day finds its deliveries dead on resume. An operator can redeliver a dead delivery by hand
  — one more attempt, same `id`.
- Redirects are not followed.
- Each hook has a delivery log (status `pending`, `delivered`, `failed` or `dead`, attempts,
  next attempt, last response code, last error) on its page. The Prometheus exposition carries
  `sanshain_webhook_deliveries_total{outcome}` and `sanshain_webhook_outbox_pending`.
- Receivers must be on the administrator's allow-list of hosts; hooks registered by an
  administrator are exempt. The list is checked at registration and again at every delivery, so
  a host removed later stops receiving (the log says why). Plain `http://` is only possible to a
  host the list names with `http://` itself.

## Related Pages

- [User Guide](user-guide.md) — Day-to-day usage: providing specs, requiring endpoints, browsing the UI.
- [Administration](administration.md) — Managing users, versions, and settings.
- [Getting Started](getting-started.md) — Installation and first-time setup.
