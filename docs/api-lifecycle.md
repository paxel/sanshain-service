# API Lifecycle & Versioning

This guide explains how to manage the evolution of your APIs in Sanshain, from initial publication to deprecation and removal.

### TL;DR
1. **Develop**: Make changes on a feature branch. Breaking changes are allowed here.
2. **Review**: Sanshain validates compatibility when you merge to a **Protected Branch** (e.g., `main`).
3. **Version**: If a change is breaking, increment the API path (e.g., `/v1` -> `/v2`).
4. **Deprecate**: Mark old endpoints as deprecated in your OpenAPI/AsyncAPI/proto spec.
5. **Retire**: Delete old endpoints only after the Dependency Graph shows zero active clients.

---

## 1. Normal Updates (Non-Breaking)
Sanshain encourages **additive changes**. Adding optional fields, new endpoints, or new enum values (depending on client tolerance) are generally safe.

- **Process**: Push your updated spec to `main`.
- **Result**: Sanshain accepts the update, versions the snippet, and notifies consumers (if using SSE/Webhooks).
- **Validation**: Sanshain's compatibility checker ensures that existing clients won't break.

## 2. Handling Breaking Changes
A breaking change is any modification that violates backward compatibility (e.g., removing a field, changing a type, renaming an endpoint).

The compatibility checker covers all three API types:

- **OpenAPI**: removed paths/methods/response codes, removed schema properties, property type changes, new required request fields.
- **AsyncAPI**: removed messages, removed payload properties, payload property type (or `$ref`) changes. Enum value and `required` changes are not analyzed.
- **gRPC/proto**: removed rpcs or messages, removed message fields, field number/type/`repeated` label changes, rpc signature changes. Enum changes are not analyzed.

Elements marked **deprecated** in the previously published spec are exempt: removing them is treated as non-breaking (see section 4 for how to mark deprecation per API type).

### Phase A: Development (Feature Branches)
You can push breaking changes to any non-protected branch.
- Clients can "opt-in" to your feature branch to test the new API.
- Sanshain tracks these as experimental versions.

### Phase B: Merge Rejection
When you try to merge a breaking change into a **Protected Branch**, Sanshain will return a `409 Conflict`.
- **Goal**: Prevent accidental breakage of production consumers.
- **Action**: Do not force the change. Instead, follow the versioning strategy below.

## 3. Versioning Strategy
When a breaking change is required, Sanshain expects you to support **side-by-side versions**.

1. **Keep the Old**: Leave the `/api/v1/resource` endpoint exactly as it is.
2. **Add the New**: Create `/api/v2/resource` with the breaking changes.
3. **Publish**: Push the spec containing *both* endpoints to `main`.
4. **Result**: Sanshain accepts the spec because it's additive. `v1` remains stable for existing clients, and `v2` becomes available for new ones.

## 4. Deprecation & Migration
Once the new version is live, you should encourage clients to migrate.

- **Annotate**: Mark the old endpoints as deprecated in your specification:
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
- **Monitor**: Check the **Dependency Graph** in the Sanshain UI. It will show exactly which clients are still "requiring" the old `v1` endpoints.
- **Communicate**: Use the dependency list to contact the owners of the consumer services.

## 5. Safe Retirement (Deletion)
The final step is removing the old code and specification.

- **Check**: Verify in the Sanshain dashboard that the "Clients" count for the old endpoint is **zero**.
- **Remove**: Delete the endpoint from your specification and push to `main`.
- **Requirement**: On protected branches, only endpoints that were **marked deprecated** in the previously published spec can be deleted. Removing a non-deprecated endpoint is rejected as a breaking change — deprecate it first (section 4), then remove it in a later update.

Retirement on a protected branch is therefore always **two publishes**: first publish the
spec *with* the deprecation marker (an ordinary compatible update), then publish the spec
*without* the element. This also applies to endpoints published before Sanshain 1.5.0: they
were stored without a deprecation flag, so re-publish them once with the marker before the
publish that removes them.

## 6. Multi-Producer Topics (AsyncAPI Message Contracts)

Unlike REST paths, which are scoped to a single service, **Kafka topic names live in a global
namespace** — topics such as an audit log or a dead-letter queue legitimately have many
producers. To keep those topics safe without penalising a producer for a change it never made,
Sanshain tracks AsyncAPI compatibility at the granularity of the **message**, not the whole
channel.

When you provide an AsyncAPI spec, every **named** `publish` message registers a *channel
message contract* keyed by `(branch, channel, message name)`:

- **Message identity** is the message `name`, falling back to its `title`. A message with
  neither has no cross-service identity: it is skipped for contract purposes and is **unsuitable
  for multi-producer topics**. Name your messages to make a shared topic safe.
- **Ownership**: the first service to publish a `(channel, message name)` on a branch owns it.
- **The owner may widen its own message** (add properties, add messages) but a breaking payload
  change — a removed non-deprecated property, or a property/`$ref` type change — is rejected with
  `409 Conflict`.
- **A different service publishing the same `(channel, message name)`** is accepted only if its
  payload schema is **semantically identical** to the owner's. Otherwise it is rejected with
  `409 Conflict` naming the owning service: *align the schema or rename your message.* The
  recommended pattern is therefore **one owner per message name** — give each producer's event a
  distinct name.

Message contracts are enforced on **all** branches, including feature branches where ordinary
endpoint breaking changes are otherwise allowed, and `force` does not bypass them. Consuming the
producer's snippet (via `/require/asyncapi`) and `$ref`-ing it from your own spec keeps a
consumer's copy aligned with the contract.

> **Direction convention.** Sanshain reads the AsyncAPI 2.x `publish`/`subscribe` keywords from
> the **application's** perspective (`publish` = *this service publishes*), matching its 3.x
> `send`/`receive` mapping. Note this is the inverse of the official 2.x specification, which
> defines the keywords from the client's perspective. Only `publish`/`send` messages register a
> contract.

---

## Summary Table

| Change Type             | Branch    | Result             | Action                                |
|-------------------------|-----------|--------------------|---------------------------------------|
| **Additive**            | Any       | Accepted           | None                                  |
| **Breaking**            | Feature   | Accepted           | Test with opt-in clients              |
| **Breaking**            | Protected | **Rejected (409)** | Use Path Versioning                   |
| **Deprecation**         | Protected | Accepted           | Monitor Dependency Graph              |
| **Deletion**            | Protected | Accepted*          | *Only for deprecated endpoints        |
| **Owned msg breaking**  | Any       | **Rejected (409)** | Widen own message only (all branches) |
| **Co-publish, differs** | Any       | **Rejected (409)** | Match owner's schema or rename        |

---

## Related Links
- [**User Guide**](user-guide.md) — Core concepts of providing and requiring.
- [**Dependency Graph**](user-guide.md#dependency-graph) — Visualizing active consumers.
- [**Troubleshooting**](troubleshooting.md) — Common 409 Conflict scenarios.
