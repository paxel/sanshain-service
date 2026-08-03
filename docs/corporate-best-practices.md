# Corporate Best Practices

This guide outlines the recommended strategies for deploying and using Sanshain in large-scale enterprise environments. Following these practices ensures flexibility, security, and maintainability as your microservice ecosystem grows.

### TL;DR
1. **Environment over Hardcoding**: Avoid putting `sanshainUrl` in `sanshain.yaml`.
2. **Centralized Variables**: Use CI/CD organization variables for the server URL and authentication tokens.
3. **Honest Semver**: Let the server propose the next version on a `409` — never work around a bump.
4. **CI Releases GA**: Set the ga switch (`SANSHAIN_GA=true`) only in your protected-branch pipelines, so only those builds publish immutable GA versions — everything else is a snapshot.
5. **LDAP/OIDC**: Integrate with your corporate identity provider for user management.

---

## 1. Flexible Configuration

Hardcoding the Sanshain Service URL in every project's `sanshain.yaml` creates a maintenance burden. If the service moves to a new domain, you would have to update hundreds of repositories.

**Best Practice**:
- Leave `sanshainUrl` out of the `sanshain.yaml` file.
- Provide the URL via the `SANSHAIN_URL` environment variable in your CI/CD platform (e.g., GitHub Org Variables, GitLab Group Variables).
- For local development, developers can set the variable in their shell profile or use build tool settings (like [**`settings.xml`**](../../SanshainMaven/docs/corporate-usage.md#using-settingsxml-recommended) for Maven).

## 2. Secure Authentication

Never check API tokens into version control.

**Best Practice**:
- Use the **API Token** feature (prefixed `san_`) for all automated integrations.
- Store the token as a **Secret** in your CI/CD system (`SANSHAIN_TOKEN`).
- Rotate tokens annually or according to your corporate security policy.

## 3. Managing Breaking Changes

Pinned Consumers can never be broken by a new version — Sanshain's job is keeping version numbers *honest*, so a Consumer deciding on an upgrade can trust what the numbers claim. A GA whose changes are breaking without a major bump is rejected with the correct proposal.

**Best Practice**:
- Publish breaking changes as a new **major** GA version; the old majors remain immutable and keep serving their pinned Consumers.
- Follow the **deprecate-then-remove** strategy described in the [API Lifecycle](api-lifecycle.md) guide.
- Use the **Dependency Graph** in the Sanshain UI to identify which teams still pin old versions of your API (**Outdated** highlights).
- Only delete an old GA version (the audited admin escape hatch) after the dependents listing shows zero pinned Consumers.

## 4. VCS Integration & Stability

Sanshain never sees your git repository — stability is declared by the build. The mapping from branches to stability belongs in the client plugin.

**Best Practice**:
- Set `SANSHAIN_GA=true` in the release pipeline (and nowhere else) so those builds publish **GA** and every other build — CI or local — publishes **snapshots**. No branch detection, no config: the pipeline is the release authority.
- Keep feature work on **snapshots**: overwritable, never compatibility-checked, expiring when unused. Consumers can opt in by pinning a snapshot version — the graph flags them as **Snapshot-pinned**.
- Reserve the explicit `stability:` override (or CI flag) for unusual setups; the branch-derived default keeps day-to-day publishing hands-off.

## 5. Infrastructure & Scalability

For production usage, ensure your Sanshain instance is robust.

**Best Practice**:
- **Database**: Use a managed PostgreSQL instance instead of the default SQLite for better concurrency and backups.
- **Monitoring**: Enable the built-in Prometheus/OpenTelemetry metrics to track service health and API usage.
- **High Availability**: Run Sanshain in a container orchestrator (Kubernetes/Nomad) with at least two replicas and a load balancer.

---

## Related Guides
- [**`sanshain.yaml` Reference**](sanshain-yaml.md) — Configuration details.
- [**CI Integration**](ci-integration.md) — Setting up pipelines.
- [**API Lifecycle**](api-lifecycle.md) — Handling versioning.
- [**Administration**](administration.md) — LDAP setup and version administration.
