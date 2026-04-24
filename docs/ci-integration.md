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

## Dry-Run Mode (PR Validation)

All three core endpoints (`/provide`, `/require`, `/require-bundle`) support a **`dry_run`** parameter. When set to `true`, the request runs full validation but **does not persist any data**:

- `/provide` with `dry_run: true` — parses the OpenAPI YAML, splits endpoints, checks for conflicts on protected branches, but stores nothing.
- `/require` with `dry_run=true` — looks up the endpoint (with fallback), but does not record a dependency.
- `/require-bundle` with `dry_run: true` — resolves all requested endpoints, but does not record any dependencies.

This is designed for **PR validation pipelines**: test whether a feature branch's contracts are valid against the main branch before allowing a merge, without polluting the database with temporary data.

### Example: Validate a Provider Spec

```bash
# In the service's CI pipeline (e.g. on PR)
curl -f -X POST http://localhost:3000/provide \
  -H "Authorization: Bearer $SANSHAIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"servicename\": \"UserService\",
    \"branch\": \"main\",
    \"openapi_yaml\": $(cat openapi.yaml | jq -Rs .),
    \"dry_run\": true
  }"
```

If the spec would cause a conflict (e.g. a DTO change on a protected branch), the request returns `409 Conflict` with a descriptive error message — the CI job fails and the developer knows exactly which endpoint is problematic.

### Example: Validate Client Dependencies

```bash
# In the client's CI pipeline (e.g. on PR)
curl -f "http://localhost:3000/require?clientname=WebApp&servicename=UserService&branch=main&path=/users&method=GET&dry_run=true" \
  -H "Authorization: Bearer $SANSHAIN_TOKEN"
```

If the endpoint doesn't exist, the response returns `404` with a message like:
```
Endpoint not found: GET /users on service 'UserService' branch 'main'
```

### Example: Validate a Bundle

```bash
curl -f -X POST http://localhost:3000/require-bundle \
  -H "Authorization: Bearer $SANSHAIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "clientname": "WebApp",
    "servicename": "UserService",
    "branch": "main",
    "endpoints": [
      {"path": "/users", "method": "GET"},
      {"path": "/users/{id}", "method": "DELETE"}
    ],
    "dry_run": true
  }'
```

If some endpoints are missing, the response returns `404` with:
```
Missing endpoints on service 'UserService' branch 'main': DELETE /users/{id}
```

## Optimistic Concurrency & Caching

Sanshain provides features to optimize CI pipelines and prevent accidental overwrites when multiple developers or automated processes update the same service branch.

### 1. Content-Based Skipping (Caching)

The server returns a `content_hash` (SHA-256) for every successful `/provide` request. If you send a specification that is identical to the current one on the server, Sanshain will:
1. Detect the identical hash.
2. Skip the database update and version increment.
3. Return the current version and hash with `202 Accepted`.

This allows CI pipelines to unconditionally "provide" the spec without worrying about unnecessary database load or version inflation.

### 2. Optimistic Concurrency Control

Each service branch has a monotonic version number. When you receive a response from `/provide`, it includes the new `version`.

To prevent overwriting concurrent changes, you can send a `base_version` in your next request. The server will only accept the update if its current version matches your `base_version`.

```bash
# Get current state (or after previous provide)
# VERSION=5

# Attempt update with base_version
curl -X POST http://localhost:3000/provide \
  -H "Content-Type: application/json" \
  -d "{
    \"servicename\": \"UserService\",
    \"branch\": \"main\",
    \"openapi_yaml\": \"...\",
    \"base_version\": $VERSION
  }"
```

If another process updated the branch in the meantime (e.g. version is now 6), the server returns `409 Conflict`. Your pipeline should then fetch the latest version, merge changes, and retry.

## Typical CI Pipeline

### Provider Pipeline (service that publishes an API)

```
PR opened → dry_run provide (validate) → merge → provide (store)
```

1. **On PR**: Run `/provide` with `dry_run: true` to validate the spec against the current main branch. If it fails, the PR is blocked.
2. **On merge to main**: Run `/provide` without `dry_run` to actually store the spec.

### Consumer Pipeline (client that depends on an API)

```
PR opened → dry_run require (validate) → merge → require (record + generate)
```

1. **On PR**: Run `/require` or `/require-bundle` with `dry_run: true` to verify all required endpoints exist. If any are missing, the PR is blocked with a descriptive error.
2. **On merge to main**: Run `/require` or `/require-bundle` without `dry_run` to record the dependency and generate client code from the returned YAML snippet.

## Descriptive Error Messages

All error responses include actionable information in the response body:

| Status | Example Message |
|--------|----------------|
| `409 Conflict` | `DTO changed for GET /users on protected branch 'main' of service 'UserService'` |
| `404 Not Found` | `Endpoint not found: GET /users on service 'UserService' branch 'main'` |
| `404 Not Found` (bundle) | `Missing endpoints on service 'UserService' branch 'main': GET /users, DELETE /users/{id}` |

These messages are designed to be shown directly in CI logs so developers can quickly identify and fix contract issues.

## GitHub Actions Example

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
              \"servicename\": \"my-service\",
              \"branch\": \"main\",
              \"openapi_yaml\": $(cat openapi.yaml | jq -Rs .),
              \"dry_run\": true
            }"
```

## Maven / Gradle Integration

Store the API token in your build tool's credentials store:

**Maven `settings.xml`:**
```xml
<server>
  <id>sanshain</id>
  <username>ignored</username>
  <password>san_xxxxxxxxxxxx</password>
</server>
```

**Gradle `gradle.properties`:**
```properties
sanshainToken=san_xxxxxxxxxxxx
```

> **Note:** Dedicated build tool plugins are under development. Check the [Sanshain GitHub organisation](https://github.com/paxel) for updates.

## Related Pages

- [User Guide](user-guide.md) — Day-to-day usage: providing specs, requiring endpoints, browsing the UI.
- [Administration](administration.md) — Managing users, protected branches, and settings.
- [Getting Started](getting-started.md) — Installation and first-time setup.
