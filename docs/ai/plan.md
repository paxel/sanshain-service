# Sanshain Service — Implementation Plan

### Documentation & Release Preparation
- [x] **Preparation for 1.4.0 Release**
  - [x] Fix broken links in dependency graph (dataset attribute bug).
  - [x] Resolve static loading screen issue (missing `hideLoader` calls).
  - [x] Disable username anonymization in audit logs and change history.
  - [x] Restructure documentation (move detailed API/Config/Benchmarks to `docs/`).
  - [x] Implement automated UI screenshot capture system (Playwright + Demo scripts).
  - [x] Implement filtered and compact unified Audit Log (Time range, Read/Write, Wildcards).
  - [x] Fix Audit filter reset bug and improve dashboard loading reliability.
  - [x] Implement strict UI Cache Control and robust reload mechanism.
  - [x] Backfill missing Audit action types and extend logging to all report types.
  - [x] Completely remove username masking logic and heal historical audit logs.
  - [x] Extend integration tests (Audit, Diff, API Tokens).
  - [x] Harden security (remove hardcoded test passwords).
  - [x] Finalize documentation with new screenshots and dark mode coverage.
  - [x] Fix UI race conditions and error handling for robust testing.

### Going Big (Enterprise & Advanced Scale)
- [ ] **Top-Layer System Switch**
  - Introduce a system-level isolation switch so that the Sanshain instance can be partitioned and used for entirely separated systems/organizations (e.g., multi-tenancy support).
- [ ] **User Roles & Groups**
  - Implement fine-grained access control allowing users/groups to contribute to or view only specific dedicated systems/namespaces.
- [ ] **Administrative/Maintenance Roles**
  - Add specialized role-based permissions to separate administrative responsibilities (e.g., separating user administration, project administration, and general read/write usage).
