# Corporate Best Practices

This guide outlines the recommended strategies for deploying and using Sanshain in large-scale enterprise environments. Following these practices ensures flexibility, security, and maintainability as your microservice ecosystem grows.

### TL;DR
1. **Environment over Hardcoding**: Avoid putting `sanshainUrl` in `sanshain.yaml`.
2. **Centralized Variables**: Use CI/CD organization variables for the server URL and authentication tokens.
3. **Path-Based Versioning**: Handle breaking changes by keeping old endpoints and adding new ones.
4. **Protected Branches**: Enable protection for `main` and `release/*` branches.
5. **LDAP/OIDC**: Integrate with your corporate identity provider for user management.

---

## 1. Flexible Configuration

Hardcoding the Sanshain Service URL in every project's `sanshain.yaml` creates a maintenance burden. If the service moves to a new domain, you would have to update hundreds of repositories.

**Best Practice**:
- Leave `sanshainUrl` out of the `sanshain.yaml` file.
- Provide the URL via the `SANSHAIN_URL` environment variable in your CI/CD platform (e.g., GitHub Org Variables, GitLab Group Variables).
- For local development, developers can set the variable in their shell profile or use build tool settings (like `~/.m2/settings.xml` for Maven).

## 2. Secure Authentication

Never check API tokens into version control.

**Best Practice**:
- Use the **API Token** feature (prefixed `san_`) for all automated integrations.
- Store the token as a **Secret** in your CI/CD system (`SANSHAIN_TOKEN`).
- Rotate tokens annually or according to your corporate security policy.

## 3. Managing Breaking Changes

Sanshain's core value is preventing breaking changes on protected branches. In a corporate environment, simply "fixing" a breaking change by forcing it is rarely the right answer, as it breaks downstream consumers you might not even know about.

**Best Practice**:
- Follow the **Side-by-Side Versioning** strategy described in the [API Lifecycle](api-lifecycle.md) guide.
- Use the **Dependency Graph** in the Sanshain UI to identify which teams are still using deprecated versions of your API.
- Only remove an old endpoint after the graph shows zero active consumers.

## 4. VCS Integration & Branch Protection

Sanshain uses branch names to track the evolution of APIs. 

**Best Practice**:
- Define **Protected Branch Patterns** in the Sanshain Admin settings (e.g., `main`, `master`, `release/*`).
- Changes to these branches are strictly validated for backward compatibility.
- Use **Feature Branches** for experimental changes. Clients can "opt-in" to a feature branch to test new API versions before they are merged.

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
- [**Administration**](administration.md) — LDAP and branch protection setup.
