# Sanshain Service — Implementation Plan

### Documentation & Developer Experience
- [x] **Streamline README & Getting Started**
  - Reduced redundancy, added TL;DR sections, and improved readability with bullet points.
- [x] **Add Troubleshooting Guide**
  - Created `docs/troubleshooting.md` for common setup and usage issues.
- [x] **AI Migration Guide & Skill**
  - Created `docs/ai-migration.md` to assist developers and AI agents in migrating services to Sanshain.
- [x] **Client Ecosystem Documentation**
  - Created a dedicated `docs/clients.md` guide and updated all references with correct links to Maven, Cargo, Go, JS, and Conan clients.
- [x] **API Lifecycle & Versioning Documentation**
  - Detailed the expected OpenAPI/AsyncAPI lifecycle, including breaking changes, side-by-side versioning, and safe retirement.
- [x] **Corporate Best Practices & Flexible Configuration**
  - Updated `sanshain.yaml` reference to mark `sanshainUrl` as optional and recommend environment-based configuration.
  - Revamped AI Migration guide and prompts to include enterprise best practices (no hardcoding, secret management).
  - Created a dedicated `docs/corporate-best-practices.md` guide for enterprise users.

### Going Big (Enterprise & Advanced Scale)
- [ ] **Top-Layer System Switch**
  - Introduce a system-level isolation switch so that the Sanshain instance can be partitioned and used for entirely separated systems/organizations (e.g., multi-tenancy support).
- [ ] **User Roles & Groups**
  - Implement fine-grained access control allowing users/groups to contribute to or view only specific dedicated systems/namespaces.
- [ ] **Administrative/Maintenance Roles**
  - Add specialized role-based permissions to separate administrative responsibilities (e.g., separating user administration, project administration, and general read/write usage).
