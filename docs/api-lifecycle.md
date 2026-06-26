# API Lifecycle & Versioning

This guide explains how to manage the evolution of your APIs in Sanshain, from initial publication to deprecation and removal.

### TL;DR
1. **Develop**: Make changes on a feature branch. Breaking changes are allowed here.
2. **Review**: Sanshain validates compatibility when you merge to a **Protected Branch** (e.g., `main`).
3. **Version**: If a change is breaking, increment the API path (e.g., `/v1` -> `/v2`).
4. **Deprecate**: Mark old endpoints as deprecated in your OpenAPI/AsyncAPI spec.
5. **Retire**: Delete old endpoints only after the Dependency Graph shows zero active clients.

---

## 1. Normal Updates (Non-Breaking)
Sanshain encourages **additive changes**. Adding optional fields, new endpoints, or new enum values (depending on client tolerance) are generally safe.

- **Process**: Push your updated spec to `main`.
- **Result**: Sanshain accepts the update, versions the snippet, and notifies consumers (if using SSE/Webhooks).
- **Validation**: Sanshain's compatibility checker ensures that existing clients won't break.

## 2. Handling Breaking Changes
A breaking change is any modification that violates backward compatibility (e.g., removing a field, changing a type, renaming an endpoint).

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

- **Annotate**: Use the `deprecated: true` flag in your OpenAPI/AsyncAPI specification for the old endpoints.
- **Monitor**: Check the **Dependency Graph** in the Sanshain UI. It will show exactly which clients are still "requiring" the old `v1` endpoints.
- **Communicate**: Use the dependency list to contact the owners of the consumer services.

## 5. Safe Retirement (Deletion)
The final step is removing the old code and specification.

- **Check**: Verify in the Sanshain dashboard that the "Clients" count for the old endpoint is **zero**.
- **Remove**: Delete the endpoint from your specification and push to `main`.
- **Cleanup**: Since no clients are using it, this is a safe, non-breaking operation.

---

## Summary Table

| Change Type     | Branch    | Result             | Action                         |
|-----------------|-----------|--------------------|--------------------------------|
| **Additive**    | Any       | Accepted           | None                           |
| **Breaking**    | Feature   | Accepted           | Test with opt-in clients       |
| **Breaking**    | Protected | **Rejected (409)** | Use Path Versioning            |
| **Deprecation** | Protected | Accepted           | Monitor Dependency Graph       |
| **Deletion**    | Protected | Accepted*          | *Only safe if client count is 0 |

---

## Related Links
- [**User Guide**](user-guide.md) — Core concepts of providing and requiring.
- [**Dependency Graph**](user-guide.md#dependency-graph) — Visualizing active consumers.
- [**Troubleshooting**](troubleshooting.md) — Common 409 Conflict scenarios.
