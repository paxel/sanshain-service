# Administration

This guide covers the admin dashboard at `/admin.html` and administrative tasks.

### TL;DR
- **Access**: Sign in at `/admin.html`. First-time login uses a random password from logs.
- **Auth**: Configure **LDAP**, **Local Users**, or **Dev Mode** in the Authentication section.
- **Versions**: Delete a version (the escape hatch from GA immutability — check the dependents first) and tune snapshot cleanup in the settings.
- **Cleanup**: Delete Producers, Consumers and stale dependencies in the management tabs.
- **Tokens**: Users create their own API tokens at `/account.html`.

---

## Accessing the Dashboard

Open `/admin.html` in your browser and sign in with an admin account. On a fresh installation the only admin account is **root** — see [Getting Started](getting-started.md) for the initial password procedure.

![Admin login screen](images/Screenshot_20260421_230832.png)

After a successful login the dashboard loads with all management sections organized into tabs.

![Admin dashboard](images/Screenshot_20260421_230857.png)

## Observability

The **Observability** tab provides real-time insights into the system's health:

- **System Stats**: Live view of CPU usage, memory consumption, and uptime.
- **Request Counters**: Total requests and 5xx failure counters for business logic.
- **Log Viewer**: An in-memory ring buffer showing the last 100 log messages with level filtering.
- **Debug Tracing**: Toggleable flags to enable detailed tracing for business logic or admin activity at runtime.

## Developer Mode

Developer Mode is configured via the unified **Authentication** section of the admin dashboard. Selecting **Dev Mode** allows public API endpoints (`/provide`, `/require`) to be accessible without any authentication. This is useful for local development and quick testing, but should not be used in production.

### Enabling Dev Mode (local only)

Because Dev Mode disables authentication entirely, it is protected by a **production safety gate** so a stray environment variable or a persisted setting left behind by configuration drift cannot silently open up a production instance. Dev Mode only becomes active when **both** of the following are true:

1. It is **requested** — either by selecting **Dev Mode** in the admin dashboard (persisted setting) or by setting `SANSHAIN_DEV_MODE=true` in the environment.
2. The safety gate `ALLOW_INSECURE_DEV_MODE=true` is set in the environment.

If Dev Mode is requested without the gate, the service **fails closed**: authentication stays enforced and the startup log emits a clear `SECURITY:` error explaining that Dev Mode was refused. Set `ALLOW_INSECURE_DEV_MODE=true` only on a trusted local machine — never in a production deployment or shared environment.

```bash
# Local development only — never set this gate in production:
ALLOW_INSECURE_DEV_MODE=true SANSHAIN_DEV_MODE=true cargo run
```

## Version Administration

The unit of administration is the **version line**: the ordered set of versions one Producer has published for one API type. Versions manage themselves for the most part — GA versions are immutable and never age-culled, snapshots expire when unused — so administration is the exceptions.

### Deleting a Version

Deleting a version is **the sole escape hatch from GA immutability** ([ADR-0003](adr/0003-versions-replace-branches.md)): it removes the version outright and frees its number for republishing. It is deliberately a heavy tool:

- **Consumers pinned to the deleted version hard-fail** (`404`) on their next require — there is no fallback. The UI shows the pinned Consumers (the dependents) before the delete is confirmed; check that listing first and get the Consumers moved off the version where possible.
- **Audited**: every delete-version writes an audit entry naming the Actor and the Consumers that were still pinned.
- **Who may**: holders of the `manage_producers` permission on any Producer, and Maintainers on their own Producers.

| Method   | Endpoint                                                        | Description                                                    |
|----------|-----------------------------------------------------------------|----------------------------------------------------------------|
| `GET`    | `/admin/producers/{name}/versions`                              | One Producer's version lines (optionally filter by `api_type`).|
| `GET`    | `/admin/producers/{name}/versions/{api_type}/{version}/dependents` | The Consumers pinned to this version — what a delete would break. |
| `DELETE` | `/admin/producers/{name}/versions/{api_type}/{version}`         | Delete the version. Audited; frees the number.                 |

### Snapshot Cleanup

Snapshots expire **use-based**: a snapshot that is neither provided nor required for `snapshot_max_age_days` is deleted by the background cleanup. Provide-age alone would delete snapshots that pinned Consumers still build against — under no-fallback that is a hard build break, so requiring a snapshot keeps it alive. GA versions are never age-culled.

- **Setting**: `snapshot_max_age_days` (default `30`, `0` disables) — `GET`/`POST /admin/settings/snapshot-max-age`, requires `manage_settings`.
- **Run now**: `POST /admin/cleanup/snapshots` triggers the expiry immediately.
- The remaining lifetime of each snapshot is visible on the Producer's version line (`expires_at`).

### Dependency Cleanup

Recorded dependencies go stale when a Consumer stops requiring an endpoint. `dependency_max_age_days` (`GET`/`POST /admin/settings/dependency-max-age`) controls when unused dependencies are removed; `POST /admin/cleanup/dependencies` runs it immediately.

## Local User Management

Sanshain supports local user accounts with the following settings in the **User Management** tab:

### Local User Registration

When **Local User Registration** is enabled, anyone can create an account via the web UI at `/account.html` or by calling `POST /auth/register`.

- **Off (default)** — only the admin can create users.
- **On** — self-registration is open.

### Auto-Approve Users

- **Off (default)** — new accounts are created in a **pending** state and must be approved by an admin before the user can log in.
- **On** — new accounts are automatically approved and can log in immediately.

This is intended for development and internal use. For production environments, consider using LDAP authentication (see below).

## Authentication Mode (OFF / Maintenance, Dev, Local, LDAP)

The **Authentication** section on the admin dashboard lets you choose how users authenticate:

| Mode                  | Description                                                                                                                                              |
|-----------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------|
| **OFF / Maintenance** | Default on new installations. Restricts all anonymous and token-based interaction with provide/require API endpoints, returning 503 Service Unavailable. |
| **Dev Mode**          | No authentication required for public API endpoints.                                                                                                     |
| **Local Users**       | Built-in user management with Argon2 password hashing.                                                                                                  |
| **LDAP**              | Delegate authentication to an external LDAP / Active Directory server.                                                                                   |

### Configuring LDAP

1. Select the **LDAP** radio button in the Authentication section.
2. Fill in the LDAP configuration form:

| Field              | Description                                                                                | Example                                 |
|--------------------|--------------------------------------------------------------------------------------------|-----------------------------------------|
| **Server URL**     | LDAP server address. Use `ldaps://` for TLS.                                               | `ldap://ldap.example.com:389`           |
| **Bind DN**        | Service account DN used to search for users.                                               | `cn=readonly,dc=example,dc=com`         |
| **Bind Password**  | Password for the service account.                                                          | *(stored encrypted, shown as `****`)*    |
| **Base DN**        | Search base for user lookups.                                                              | `dc=example,dc=com`                     |
| **User Filter**    | LDAP filter to find users. `{username}` is replaced with the login name.                   | `(uid={username})`                      |
| **Admin Group DN** | Members of this group get the `admin` role. Saved as a directory group in the group mapping, where you can attach further roles or add more groups. Leave empty to disable. | `cn=admins,ou=groups,dc=example,dc=com` |

3. Click **Test Connection** to verify that Sanshain can reach the LDAP server and bind with the service account.
4. Click **Save Authentication Settings** to apply.

> **Using `ldaps://` with a private or corporate CA?** Mount the CA certificates
> and set `EXTRA_CA_CERTS_DIR` to that directory — see
> [Additional CA certificates](configuration.md#additional-ca-certificates).
> Without it, **Test Connection** fails on certificate verification against a CA
> the platform does not already trust.

### How LDAP Login Works

When LDAP mode is active:

1. Sanshain binds to the LDAP server with the configured service account.
2. It searches for the user using the **User Filter** under the **Base DN**.
3. If found, it attempts a bind as the user with the provided password.
4. On success, a **shadow account** is auto-provisioned in the local database (if it doesn't already exist). This allows sessions and API tokens to work exactly as with local users.
5. If an **Admin Group DN** is configured, the user's `memberOf` attribute is checked to determine admin status.

### API Endpoints

The auth configuration can also be managed via the REST API:

- `GET /admin/auth-config` — Returns the current auth mode and LDAP configuration (password redacted).
- `PUT /admin/auth-config` — Set the auth mode and (optionally) LDAP configuration:
  ```json
  {
    "auth_mode": "ldap",
    "ldap_config": {
      "server_url": "ldap://ldap.example.com:389",
      "bind_dn": "cn=admin,dc=example,dc=com",
      "bind_password": "secret",
      "base_dn": "dc=example,dc=com",
      "user_filter": "(uid={username})",
      "admin_group": "cn=admins,ou=groups,dc=example,dc=com"
    }
  }
  ```
- `POST /admin/auth-config/test` — Test LDAP connectivity with the provided configuration.

> **Note:** Submitting `"bind_password": "****"` in a PUT or test request preserves the previously stored password.

## User Management

The **Users** section lists every registered user with their status:

| Badge        | Meaning                                            |
|--------------|----------------------------------------------------|
| **admin**    | The user has administrator privileges.             |
| **approved** | The user can log in and use the service.           |
| **pending**  | The user registered but has not yet been approved. |

Available actions:

- **Approve** — grant a pending user access to the service.
- **Delete** — permanently remove a user and revoke all their sessions and API tokens.

## Roles and Groups

Authorisation is expressed in **permissions** — the unit every check tests. **Roles** are fixed
bundles of permissions, defined in the service rather than composed by an operator, so what a role
confers cannot drift from what the code enforces.

| Role           | Confers                                                                      |
|----------------|------------------------------------------------------------------------------|
| `admin`        | Every permission. This is the role an existing administrator account holds.  |
| `user_manager` | User administration and role/group administration, and nothing else.         |
| `viewer`       | Read access to the audit log and observability.                              |
| `maintainer`   | Producer administration (`manage_producers`) — **scoped**.                   |
| `releaser`     | Publishing GA versions (`release_ga`) — **admin-guarded**.                   |

`maintainer` cannot be granted instance-wide: it is meaningless without the set of Producers it is
over, so it is assigned as a scope rather than granted as a role.

`releaser` and `admin` are **admin-guarded**: only an admin (or root) may grant or revoke them, or
move them on or off a group. For `admin` that closes self-escalation; for `releaser` it keeps
`manage_roles` from being a side door to release rights.

### Releasing

Publishing with `stability: ga` — a fresh GA, a promotion of a snapshot, or even an idempotent GA
re-provide — requires the `release_ga` permission; everything else answers `403` and shows up in the
audit log as a `VERSION_REJECTED` entry with reason `ga_requires_releaser`. Snapshots stay open to
every authenticated caller.

Grant `releaser` to whatever performs your releases — typically the CI user whose token sets
`sanshain.ga=true`. Admins and root hold the permission implicitly. Maintainers do **not**: a
maintainer can delete a GA version of their Producer (remediation), but releasing is deliberately
the pipeline's job.

### Groups

A **group** is a set of users that roles attach to. Groups carry a source, which records where their
membership comes from:

- **native** — Sanshain's own. Membership is yours to edit.
- **ldap** — mirrors a directory group. Membership belongs to the directory and is not stored here;
  only the roles you attach to it are. Membership is re-read from the directory through the configured
  service account, cached briefly — see
  [Directory group caching](configuration.md#directory-group-caching) — so a change in the directory
  takes effect without the user signing in again.

A native and a directory group may share a name without colliding — they are distinct entities, and
the UI shows the origin.

### Maintainers

A **maintainer** is responsible for a set of Producers. It is an assignment rather than a role,
because it means nothing without the Producers it is over — so it is never granted instance-wide.

A user or a group can be assigned. Group assignment is what makes this scale: put a team's group on
the Producers that team owns, and responsibility follows membership.

A Producer-scoped action admits either the matching instance-wide permission **or** maintainership of
that Producer. So an administrator can act on any Producer, while a maintainer can act on theirs and
is refused on everybody else's. This covers Producer administration — deleting one of its versions
(the GA escape hatch), or the Producer itself.

### API Endpoints

| Method   | Endpoint                                  | Description                                     |
|----------|-------------------------------------------|-------------------------------------------------|
| `GET`    | `/admin/roles`                            | The role catalogue and what each role confers.  |
| `GET`    | `/admin/users/{id}/roles`                 | Roles granted directly to a user.               |
| `POST`   | `/admin/users/{id}/roles`                 | Grant a role. Body: `{"role": "user_manager"}`. |
| `DELETE` | `/admin/users/{id}/roles/{role}`          | Revoke a role.                                  |
| `GET`    | `/admin/groups`                           | All groups with their origin, roles and members.|
| `POST`   | `/admin/groups`                           | Create a native group. Body: `{"name": "..."}`. |
| `PUT`    | `/admin/groups/{id}`                      | Rename it, replace its roles, or both.          |
| `DELETE` | `/admin/groups/{id}`                      | Delete a group and its grants.                  |
| `POST`   | `/admin/groups/{id}/members`              | Add a member. Body: `{"user_id": 3}`.           |
| `DELETE` | `/admin/groups/{id}/members/{user_id}`    | Remove a member.                                |
| `GET`    | `/admin/maintainers`                      | Every Producer with its maintainers, in one response. |
| `GET`    | `/admin/producers/{name}/maintainers`     | Users and groups maintaining a Producer. Readable by that Producer's maintainers too. |
| `POST`   | `/admin/producers/{name}/maintainers`     | Assign one. Body: `{"user_id": 3}` **or** `{"group_id": 1}`. |
| `DELETE` | `/admin/producers/{name}/maintainers/users/{user_id}`   | Unassign a user.                  |
| `DELETE` | `/admin/producers/{name}/maintainers/groups/{group_id}` | Unassign a group.                 |
| `GET`    | `/admin/users/{id}/maintains`             | The Producers a user is responsible for.        |

## Producer Management

The **Producers** section lists all Producers that have provided at least one specification, with metadata and their full version lines — version, stability, endpoint count, and (for snapshots) the use-based expiry.

Per version-line entry you can:

- **View** its endpoints, the full provided document, and a diff between any two versions of the line.
- **Delete** the version — see [Deleting a Version](#deleting-a-version); the dependents are shown before confirming.

At the Producer level:

- **Delete** — removes the Producer and **cascades** to all its version lines, endpoints, and related Consumer dependencies.

A confirmation dialog appears before executing any deletion.

For a detailed view of a Producer's version lines and endpoints, use the Producers page (`/producers.html`) instead.

## Consumer Management

The **Consumers** section lists all Consumers that have recorded at least one dependency via `/require`.

- **Delete** — removes the Consumer and all its recorded dependencies. A confirmation dialog appears before deletion.

## Changing Your Password

Click the **Change Password** button in the top navigation bar to open the password dialog.

Enter your current password and a new password, then click **Update Password**. The change takes effect immediately; existing sessions remain valid.

## Database Configuration

Sanshain supports two database backends: **SQLite** (default) and **PostgreSQL**. The backend is selected automatically based on the `DATABASE_URL` environment variable:

| URL prefix                       | Backend    |
|----------------------------------|------------|
| `sqlite:` or not set             | SQLite     |
| `postgres://` or `postgresql://` | PostgreSQL |

The current database backend and a masked connection URL are visible in the admin dashboard under **Database Configuration**. This section is read-only — to switch backends, change the `DATABASE_URL` environment variable and restart the service.

For detailed setup instructions, see [Getting Started — PostgreSQL](getting-started.md#installation-with-docker-postgresql).

## Related Pages

- **Account** (`/account.html`) — manage your own password and API tokens.

  ![Account page with token management](images/Screenshot_20260421_230948.png)
