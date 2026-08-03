# Sanshain Service — Implementation Plan

### Going Big (Enterprise & Advanced Scale)
- [ ] **Top-Layer System Switch**
  - Introduce a system-level isolation switch so that the Sanshain instance can be partitioned and used for entirely separated systems/organizations (e.g., multi-tenancy support).
- [x] **User Roles & Groups** — shipped in 2.0.0; issues #13, #14, #15, #18, #19, #20 closed.
  - Permissions are the unit every authorisation check tests; roles are fixed bundles of them,
    defined in Rust rather than composed by operators. Fixed bundles are what allow a build-time
    check to reason about who can reach a route.
  - Groups carry roles, and a user's effective permissions are the union of direct grants and grants
    via the groups they belong to.
  - Groups come from two origins and must not collide: those mirrored from the directory Sanshain
    authenticates against, and Sanshain's own. A single group table with an origin discriminator
    keeps every permission check identical regardless of where a group came from.
  - Directory membership stays the directory's: it is re-read through the existing service account
    on a short cache TTL, so a group change takes effect without the user logging out. Sanshain's
    own membership is read live.
  - Tracked as GitHub issues #13, #14, #15, #18, #19, #20.
- [x] **Administrative/Maintenance Roles** — shipped in 2.0.0; issues #16, #17, #24 closed.
  - Separate the two mechanisms rather than flattening them into one role list. `admin`,
    `user_manager` and `viewer` are instance-wide. **Maintainer** is not a role but a scope: an
    assignment of a user or group to a set of Producers, meaningless without them.
  - A Producer-scoped action admits either the global permission or maintainership of that Producer.
  - **Root** holds every permission — including permissions added later — and is configured outside
    the database, so no stored grant, UI action or direct SQL update can revoke it.
  - The administrative HTTP surface gets its own OpenAPI contract, separate from the Producer and
    Consumer contract, because the two have different audiences and different stability promises.
  - Tracked as GitHub issues #16, #17, #24.

### Onboarding and change review — removed in 2.0
- ~~**Producer onboarding**~~ (issue #21) and ~~**Pending specs and review**~~ (issues #22, #23)
  were shipped and then removed by the 2.0 versions-replace-branches rework: without protected
  branches every rejection is self-service (409 + `proposed_version`), so there is nothing to
  onboard around or review. See `docs/adr/0003-versions-replace-branches.md` (supersedes ADR 0002).
