# Sanshain Service — Implementation Plan

### Documentation & Release Preparation
- [x] **Preparation for 1.4.0 Release**
  - [x] Restructure documentation (move detailed API/Config/Benchmarks to `docs/`).
  - [x] Implement automated UI screenshot capture system (Playwright + Demo scripts).
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
