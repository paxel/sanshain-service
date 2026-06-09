# Sanshain Service — Implementation Plan

### Deployment
- [ ] **Kubernetes Manifests**
  - Provision standard, production-ready manifests including `Deployment` (with replica configuration, health checks, resource limits), `Service` (ClusterIP/NodePort), `Ingress` (routing rules for Axum endpoints and static assets), `ConfigMap` (app settings, databases URLs, features), and `Secret` (database credentials, JWT secret keys, LDAP bind password).
- [ ] **Helm Chart**
  - Create a standard Helm chart to facilitate packaged Kubernetes deployments, allowing customized configuration management, templating for environment-specific values, and simplified installation/upgrades.
- [ ] **Reverse Proxy TLS Configuration & Documentation**
  - Configure and document a robust reverse proxy setup (e.g., NGINX, Caddy, or Traefik) that handles TLS termination, secure HTTPS redirection, and proper header forwarding (e.g., `X-Forwarded-For`, `X-Real-IP`).

### Observability (Advanced)
- [ ] **OpenTelemetry Tracing**
  - Integrate OpenTelemetry tracing to track and monitor request flows, end-to-end performance, and service/database call latencies across service boundaries.

### Custom Dependency Graph Visualization (Polish)
- [ ] **Edge Bundling / Merging**
  - Implement edge bundling/merging at endpoint entry and exit points in the interactive dependency graph to reduce visual clutter and overlapping lines on highly interconnected graphs.
- [ ] **Export as PNG**
  - Add native support in the UI for exporting and downloading the rendered dependency graph as a high-resolution PNG image (expanding on the existing SVG export feature).

### Web Frontend (Advanced)
- [ ] **Server-Sent Events (SSE) for Live Updates**
  - Implement Server-Sent Events for the `/require` long-polling endpoint, enabling live, push-based dependency and spec updates in the client UI without constant HTTP polling.
- [ ] **WebSocket Support**
  - Explore WebSocket-based bidirectional communication to support real-time interactive collaboration features in the future.

### AsyncAPI Semantics & Sync Concurrency
- [ ] **Problem 7: Semantic Versioning (SemVer) for Specs**
  - **Context**: Currently, Sanshain uses simple monotonic integers for specification versions.
  - **Goal**: Support user-provided semantic versioning (SemVer) or introduce automatic detection of `MAJOR`/`MINOR`/`PATCH` changes based on backward-compatibility analysis.

### Going Big (Enterprise & Advanced Scale)
- [ ] **Top-Layer System Switch**
  - Introduce a system-level isolation switch so that the Sanshain instance can be partitioned and used for entirely separated systems/organizations (e.g., multi-tenancy support).
- [ ] **User Roles & Groups**
  - Implement fine-grained access control allowing users/groups to contribute to or view only specific dedicated systems/namespaces.
- [ ] **Administrative/Maintenance Roles**
  - Add specialized role-based permissions to separate administrative responsibilities (e.g., separating user administration, project administration, and general read/write usage).
- [ ] **Manual Spec Editing**
  - Mark endpoints as "external" or consumed by external systems.
  - Add external services directly in the UI.
  - Define and write YAML specifications for external services manually in the UI.
- [ ] **Eye Candy & Polish**
  - Allow users to choose custom icons/symbols for services.
  - Group related services into logical clusters/domains in the visual graph.
