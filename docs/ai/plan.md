# Sanshain Service — Implementation Plan

### Custom Dependency Graph Visualization (Polish)
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
