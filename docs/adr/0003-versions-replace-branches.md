---
status: accepted
---

# Producer-declared versions replace branches

Sanshain 2.0 removes the branch model entirely. The unit of publication is a **version line per
(Producer, API type)**: each Provide carries a strict `MAJOR.MINOR.PATCH` version read from the
spec file itself (`info.version`; for proto a mandatory `// sanshain-version:` comment, rejected
when missing or conflicting) and a declared **stability** — `snapshot` (mutable, last-writer-wins)
or `ga` (immutable forever). Consumers pin an exact version; resolution is GA, else Snapshot, else
an immediate failure. This is a clean break of the wire contract: no `/v2` namespace, no
compatibility layer, every client plugin upgrades.

## Why

Branches were a claim about the Producer's git repository that Sanshain could never verify — it
has no git access. Everything downstream of that claim accumulated machinery to compensate:
protected-branch patterns, source-protected-branch stickiness, pull-from overrides, inheritance
with four resolution states, ghost nodes, stale-branch cleanup, branch protection and nuke admin.
The versioning *intent* (is this work-in-progress or a release?) was being inferred from branch
names when it is something the caller can simply state. Declaring stability directly, and making
the version part of the artifact itself, deletes the inference and most of the machinery:

- **GA immutability is the concurrency control.** "Same version, different content" is rejected
  with a proposed next version (breaking → major, additive → minor, shape-identical → patch), so
  the forgot-to-bump mistake is caught at the door instead of surfacing as a mystery later.
- **Mislabel checking replaces breaking-change gating.** Pinned consumers cannot be broken by a
  new version, so holding breaking changes for review is pointless. What remains worth catching is
  a semver lie: a GA whose changes against the highest GA below it are breaking without a major
  bump is rejected. Snapshots are never compat-checked. Onboarding and the pending-spec review
  flow (ADR 0002) are removed — every rejection now has a self-service remedy.
- **No fallback, no long-poll.** Exact pins are written by a human after the version exists, so
  the race the 1.x long-poll served (implicit same-branch following) is gone by construction. A
  missing pinned version is a config error and fails in milliseconds (`404`); a version that lacks
  the endpoint is a deliberate absence (`410`, unchanged from ADR 0001's insight).
- **Resolution never creates graph entities.** Dependencies are recorded only on successful
  requires; a failed require cannot compile, so nothing broken ships and no ghost nodes exist.

## Considered and rejected

- **A `sanshain.yaml` version field for proto.** Proto has no in-band version slot; a config field
  is explicit but breaks the uniform rule "the version lives in the spec file". A *mandatory*
  comment fails loudly on absence or typo, and the regex-based proto splitter already carries
  file-level comments into the split output, so the version travels in the artifact for free.
- **Maven-style `-SNAPSHOT` suffixes.** Stability is a state of a stored version, declared on the
  Provide; encoding it in the string would create a second, contradictable source of truth.
- **Version ranges / `latest` pins.** Reintroduce the served-spec-changing-underneath-you surprise
  the redesign exists to kill, and make the graph's "outdated" highlight vacuous. Upgrade pressure
  belongs in the UI, not in resolution.
- **Per-Actor snapshot locking.** Rejecting a different Actor's overwrite of a snapshot (forcing
  them to a fresh number) protects concurrent developers — until the same human provides locally
  with a personal token and via CI with the Jenkins token, and locks themselves out. Ownership by
  token cannot see who is behind the token. Snapshots are last-writer-wins with *visibility*
  instead: last provider in the UI, every overwrite audited.
- **Retaining `baseVersion` optimistic concurrency.** It guarded a branch's spec because the
  branch had no other coordination point. Two people editing one spec file coordinate in git; the
  next publish after their merge converges the snapshot.
- **A parallel `/v2` namespace.** Buys coexistence that was explicitly decided against — running
  the branch resolver next to the version resolver would mean maintaining both mental models,
  which is the cost the rewrite removes.

## Consequences

- **`api.yaml` breaks its stability promise once.** Provide routes gain required `stability` and
  lose `branch`/`baseVersion`; require routes gain required `version` and lose `branch`,
  pull-from and `timeout`; `/branches/*` is replaced by `GET /producers/{producername}/versions`.
  Migration: adapt the Maven plugin, republish each producer under a real version (snapshot via a
  feature branch first), repoint the consumers that build against Sanshain, then cut over.
- **Existing spec data is dropped, not migrated.** Branch history has no honest mapping onto
  semver lines. The 2.0 SQL migration drops spec/branch/dependency/pending tables; users, tokens,
  groups, roles, maintainer scopes, settings and the audit log survive. The operator's escape
  hatch is a documented pre-upgrade backup.
- **GA permanently claims its number.** Promotion deletes only the same-numbered snapshot; later
  snapshot provides for that number are rejected; lower never-GA'd snapshot numbers stay legal
  (hotfix prep). The sole escape hatch is an audited delete-version action, held by admins and by
  Maintainers for their own Producers, showing pinned consumers before confirming.
- **Snapshot cleanup is use-based.** A snapshot expires when neither provided nor required for
  `snapshot_max_age_days` (default 30, `0` disables) — provide-age alone would delete snapshots
  that pinned consumers still build against, which under no-fallback is a hard build break. GA is
  never age-culled. `dependency_max_age_days` survives unchanged.
- Producers whose `info.version` is currently not strict semver are rejected on their first 2.0
  provide until they fix it — deliberate, since the field is now load-bearing.
- ADR 0001 and ADR 0002 are superseded: 0001's branch-authority model has no branches to govern
  (its `Absent`-is-deliberate and reads-create-nothing insights live on here), and 0002's held
  specs have no review flow to feed.
