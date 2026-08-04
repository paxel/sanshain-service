# Troubleshooting Guide

Common issues encountered when setting up or using Sanshain Service and their solutions.

### TL;DR
- **Lost Admin Password?** Reset it by deleting `sanshain.db` (for SQLite) or clearing the `users` table (Postgres) and restarting.
- **API Returning 401?** Send a valid `Authorization: Bearer <token>` header, or enable **Developer Mode** (which additionally requires the `ALLOW_INSECURE_DEV_MODE=true` safety gate — see below).
- **Provide Rejected with 409?** The body carries `proposed_version` — set your spec's version to it and republish.

---

## Installation & Startup

### Port 3000 is already in use
**Issue**: The service fails to start with an error like `Address already in use`.
**Solution**: Change the port using the `PORT` environment variable:
```bash
PORT=3080 ./sanshain_service
```

### Database Connection Failure
**Issue**: `Failed to connect to database`.
**Solution**: 
- **SQLite**: Ensure the directory for `DATABASE_URL` exists and is writable.
- **Postgres**: Verify the connection string and ensure the Postgres server is reachable from the Sanshain host/container.

---

## Authentication & Access

### Lost the initial root password
**Issue**: You missed the password in the logs or forgot it.
**Solution**: 
1. Stop the service.
2. **SQLite**: Delete the `sanshain.db` file (WARNING: this deletes all data).
3. **Postgres**: Run `DELETE FROM users WHERE username = 'root';` in your database.
4. Restart the service. A new root account with a new random password will be created.

### "Forbidden" or "Unauthorized" on API calls
**Issue**: `POST /provide` or `GET /require` returns 401 or 403.
**Solution**:
1. Create an **API Token** in your Account settings (`/account.html`) and include it in your request: `Authorization: Bearer san_...` (recommended).
2. OR, for local development only, enable **Developer Mode** for unauthenticated access. Dev Mode fails closed unless you **also** start the service with the safety gate `ALLOW_INSECURE_DEV_MODE=true`; without it, requests stay locked and a `SECURITY:` error is logged at startup. See [Developer Mode](administration.md#enabling-dev-mode-local-only) for details. Never set this gate in production.

---

## Specification Management

### 409 Conflict: Provide rejected by the version rules
**Issue**: Your Provide was rejected — the version already exists as GA with different content, the changes are breaking without a major bump, or you sent a snapshot for a number that has gone GA.
**Solution**: Every rejection is self-service. The response body carries `proposed_version` — the next free version, bumped by what actually changed. Set your spec's version (`info.version`, or the `// sanshain-version:` comment for proto) to it and republish. See [**API Lifecycle**](api-lifecycle.md) for the full rules.

### 404 on require: Unknown version
**Issue**: `GET /require` returns 404 even though the Producer exists.
**Solution**: The pinned `version` does not exist on the server in either stability — a Pin configuration error. There is no fallback and nothing waits. Check `GET /producers/{name}/versions` for what actually exists, and fix the Pin in your `sanshain.yaml`. Also check whether the version was a snapshot that expired unused, or was deleted by an admin.

### 410 on require: Endpoint absent
**Issue**: `GET /require` returns 410.
**Solution**: The pinned version exists but deliberately does not include that endpoint — provided specs are complete, so absence is a definitive no. Check the `path`/`method` spelling against what the Producer's version actually contains, or pin a version that has the endpoint.

### Spec splitting fails
**Issue**: The service returns a 400 error when uploading a specification.
**Solution**:
- Ensure the YAML is valid.
- Ensure the version is strict `MAJOR.MINOR.PATCH` — `info.version` for OpenAPI/AsyncAPI, exactly one `// sanshain-version:` comment for proto. No `v` prefix, no suffixes.
- For OpenAPI: Ensure it's version 3.0 or 3.1.
- For AsyncAPI: Ensure it's version 2.x or 3.x.
- For gRPC: Ensure it's a valid `.proto` file.

---

## Web Dashboard

### Dependency Graph is empty
**Issue**: No nodes or edges are shown in the graph.
**Solution**:
- Ensure you have both **provided** specifications and **required** endpoints.
- Dependencies are only tracked when a client successfully calls `/require` or `/require-bundle`.
- Use the `./demo.sh` script to see what a populated graph looks like.

### Graph Export (PNG) fails
**Issue**: Clicking "Download PNG" does nothing or shows a security error.
**Solution**:
- Ensure you are using a modern browser (Chrome, Firefox, Safari).
- If running behind a reverse proxy, ensure `blob:` URLs are allowed in the Content Security Policy (CSP) headers. Sanshain sets these correctly by default.
