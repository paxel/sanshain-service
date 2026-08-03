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

## Related Pages

- [User Guide](user-guide.md) — Day-to-day usage: providing specs, requiring endpoints, browsing the UI.
- [Administration](administration.md) — Managing users, versions, and settings.
- [Getting Started](getting-started.md) — Installation and first-time setup.
