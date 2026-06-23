# Sanshain Service — Implementation Plan

### Custom Dependency Graph Visualization (Polish)
- [x] **Export as PNG**
  - Add native support in the UI for exporting and downloading the rendered dependency graph as a high-resolution PNG image.
- [x] **Interactive Graph Popups**
  - Implement custom HTML tooltips on nodes and edges with detailed service/client/endpoint information and direct links to specifications.

### Web Frontend (Advanced)
- [ ] **Server-Sent Events (SSE) for Live Updates**
  - Implement SSE to broadcast spec update notifications to the UI, enabling real-time graph and report refreshes.
- [ ] **Audit View & Change History**
  - Create a dedicated Audit page showing a timeline of specification updates, including diffs and author metadata per branch.
- [ ] **WebSocket Support**
  - Explore WebSocket-based bidirectional communication for future real-time collaboration features.

### OpenAPI Semantics & Contract Enforcement
- [ ] **Enhanced Breaking Change Detection**
  - Detect removed paths/operations and new required fields in request bodies as breaking changes to prevent accidental breakages on protected branches.
- [ ] **Deprecation Support & Client Warnings**
  - Automatically detect `deprecated: true` from OpenAPI specs and inject `X-Sanshain-Deprecated` headers in `/require` responses. Show visual warnings in the UI.
- [ ] **Semantic Versioning (SemVer) for Specs**
  - Support SemVer (MAJOR/MINOR/PATCH) for specification versions, with automatic incrementing based on change impact analysis.

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
