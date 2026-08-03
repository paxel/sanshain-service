# API Lifecycle & Versioning

This guide explains how to manage the evolution of your APIs in Sanshain, from the first snapshot to deprecation and removal. The unit of evolution is the **version line**: the ordered set of `MAJOR.MINOR.PATCH` versions one Producer has published for one API type.

### TL;DR
1. **Iterate**: Develop against **snapshot** versions — overwritable, last writer wins, expire when unused.
2. **Release**: Provide the same number as **GA** — it is promoted in place and becomes immutable.
3. **Bump**: When a GA Provide is rejected `409`, the body proposes the correct next version — set it in your spec and republish.
4. **Deprecate**: Mark old elements as deprecated in your spec; removing deprecated elements later is non-breaking.
5. **Retire**: Old GA versions keep serving pinned Consumers until they move their Pin; the Dependency Graph shows who still depends on what.

---

## 1. Iterating on Snapshots
While an API is in flux, provide it as a snapshot — which is simply the default: every build publishes `snapshot` unless the ga switch is set (see [sanshain.yaml](sanshain-yaml.md#how-stability-is-decided)).

- **Overwritable**: Re-providing the same snapshot number replaces it wholesale. Last writer wins; every overwrite is audited and the previous provider is named.
- **Never compatibility-checked**: Break whatever you like between snapshot overwrites.
- **Consumers can opt in**: A Consumer that pins your snapshot version builds against work-in-progress — the graph flags it as **Snapshot-pinned**.
- **Expiry**: A snapshot that is neither provided nor required for `snapshot_max_age_days` (default 30) is cleaned up. Use keeps it alive.

## 2. Going GA (Promotion)
When the version is ready, provide the same number with `stability: ga`:

- **Promotion in place**: The snapshot entry becomes GA; the snapshot content is gone.
- **Immutable forever**: Re-providing identical content is a no-op; different content is rejected with a proposed next version.
- **The number is permanently claimed**: No snapshot may ever exist for a GA'd number again. Lower never-GA'd numbers stay legal (e.g., preparing a hotfix `1.2.1` while `1.3.0` is GA).
- **Never age-culled**: GA versions survive until deliberately deleted.

## 3. Choosing the Next Version (the Server Proposes)
You never have to guess the right bump. A GA Provide is rejected `409 Conflict` with a `proposed_version` when:

- **Forgot to bump**: The number already exists as GA with different content. The proposal is the next free number, bumped by what actually changed — breaking → major, additive → minor, shape-identical → patch.
- **Semver lie**: The changes relative to the highest GA below it are breaking without a major bump. The proposal is the correct major.
- **Reusing a released number as snapshot**: A snapshot Provide for a GA'd number is rejected — pick the proposed next number.

Every rejection is self-service: set your spec's version (`info.version`, or the `// sanshain-version:` comment for proto) to the proposal and republish.

The breaking-change classifier covers all three API types:

- **OpenAPI**: removed paths/methods/response codes, removed schema properties, property type changes, new required request fields.
- **AsyncAPI**: removed messages, removed payload properties, payload property type (or `$ref`) changes. Enum value and `required` changes are not analyzed.
- **gRPC/proto**: removed rpcs or messages, removed message fields, field number/type/`repeated` label changes, rpc signature changes. Enum changes are not analyzed.

Elements marked **deprecated** in the previous GA are exempt: removing them is treated as non-breaking (see section 4 for how to mark deprecation per API type).

> **Note**: Pinned Consumers are never broken by a new version — they keep getting exactly what they pinned. The checks above exist to keep version numbers honest, so a Consumer reading `2.4.0 → 2.5.0` can trust that the upgrade is compatible.

## 4. Deprecation & Migration
Once a new major version is GA, encourage Consumers to move their Pins.

- **Annotate**: Mark the old elements as deprecated in your specification:
  - **OpenAPI**: `deprecated: true` on the operation (and/or on individual schema properties).
  - **AsyncAPI**: `deprecated: true` (or `x-deprecated: true`) on the `publish`/`subscribe` operation, message, or payload property.
  - **gRPC/proto**: `option deprecated = true;` inside the rpc or message body, or `[deprecated = true]` on a field.

Deprecation is strictly **per element** — there is no whole-spec switch. A root-level
`deprecated: true` in an AsyncAPI document or a file-level `option deprecated = true;` in a
`.proto` file is ignored. The marker must also sit **at the level of the thing you later
remove**:

  - Deleting a whole endpoint (OpenAPI operation, AsyncAPI channel operation, proto rpc)
    requires the marker on that operation/rpc itself.
  - Removing a message requires the marker on that message; removing a payload property or
    proto field requires the marker on that property/field. Deprecating an operation does
    *not* exempt removing individual fields from its payload.
  - Proto nested messages are independent: deprecating `Outer` does not cover `Outer.Inner`.
- **Monitor**: Check the **Dependency Graph** in the Sanshain UI. It highlights **Outdated** Pins (below your latest GA) and shows exactly which Consumers still pin the old versions.
- **Communicate**: Use the dependency list to contact the owners of the consumer services.

## 5. Safe Retirement (Removal)
Removing an element from your API is always **two GA releases** within the line:

1. Publish a version *with* the deprecation marker (an ordinary compatible update — a minor bump).
2. Publish a later version *without* the element. Because the element was deprecated in the previous GA, the removal is classified non-breaking, so a minor bump suffices.

Consumers pinned to older GA versions are unaffected — those versions are immutable and keep serving. Retirement of the *versions themselves* is a separate concern:

- **Old GA versions** stay available forever by default. That is a feature: a Pin never rots out from under a Consumer.
- **The escape hatch** is the audited **delete-version** admin action (admins, and Maintainers for their own Producers). It frees the number, and Consumers still pinned to it hard-fail (`404`) on their next require — the UI shows the pinned Consumers before confirming. See [Administration](administration.md#version-administration).

## 6. Multi-Producer Topics (AsyncAPI Message Contracts)

Unlike REST paths, which are scoped to a single service, **Kafka topic names live in a global
namespace** — topics such as an audit log or a dead-letter queue legitimately have many
producers. To keep those topics safe without penalising a producer for a change it never made,
Sanshain tracks AsyncAPI compatibility at the granularity of the **message**, not the whole
channel.

When you provide an AsyncAPI spec as **GA**, every **named** `publish` message registers a
*channel message contract* keyed by `(channel, message name)`:

- **Message identity** is the message `name`, falling back to its `title`. A message with
  neither has no cross-service identity: it is skipped for contract purposes and is **unsuitable
  for multi-producer topics**. Name your messages to make a shared topic safe.
- **Ownership**: the first Producer to publish a `(channel, message name)` owns it.
- **The owner may widen its own message** (add properties, add messages) but a breaking payload
  change — a removed non-deprecated property, or a property/`$ref` type change — is rejected with
  `409 Conflict`.
- **A different Producer publishing the same `(channel, message name)`** is accepted only if its
  payload schema is **semantically identical** to the owner's. Otherwise it is rejected with
  `409 Conflict` naming the owning Producer: *align the schema or rename your message.* The
  recommended pattern is therefore **one owner per message name** — give each producer's event a
  distinct name.

Message contracts are enforced on **GA provides only** — snapshots are never contract-checked, so
feature work stays friction-free. Consuming the producer's snippet (via `/require/asyncapi`) and
`$ref`-ing it from your own spec keeps a consumer's copy aligned with the contract.

> **Direction convention.** Sanshain reads the AsyncAPI 2.x `publish`/`subscribe` keywords from
> the **application's** perspective (`publish` = *this service publishes*), matching its 3.x
> `send`/`receive` mapping. Note this is the inverse of the official 2.x specification, which
> defines the keywords from the client's perspective. Only `publish`/`send` messages register a
> contract.

---

## Summary Table

| Situation                                   | Stability | Result             | Action                                   |
|---------------------------------------------|-----------|--------------------|------------------------------------------|
| **Any change, new or same snapshot number** | Snapshot  | Accepted           | Iterate freely; overwrites are audited   |
| **Same GA number, identical content**       | GA        | Accepted (no-op)   | None                                     |
| **Same GA number, different content**       | GA        | **Rejected (409)** | Republish as `proposed_version`          |
| **Breaking change without major bump**      | GA        | **Rejected (409)** | Republish as the proposed major          |
| **Snapshot for a GA'd number**              | Snapshot  | **Rejected (409)** | Use the proposed next number             |
| **Removing a deprecated element**           | GA        | Accepted (minor)   | Deprecate first, remove one GA later     |
| **Owned msg breaking (AsyncAPI)**           | GA        | **Rejected (409)** | Widen own message only                   |
| **Co-publish, schema differs (AsyncAPI)**   | GA        | **Rejected (409)** | Match owner's schema or rename           |

---

## Related Links
- [**User Guide**](user-guide.md) — Core concepts of providing and requiring.
- [**Dependency Graph**](user-guide.md#dependency-graph) — Visualizing active consumers.
- [**Troubleshooting**](troubleshooting.md) — Common 409 Conflict scenarios.
