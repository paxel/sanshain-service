# Administration

This guide covers the admin dashboard at `/admin.html` and administrative tasks.

### TL;DR
- **Access**: Sign in at `/admin.html`. First-time login uses a random password from logs.
- **Auth**: Configure **LDAP**, **Local Users**, or **Dev Mode** in the Authentication section.
- **Protection**: Manage **Protected Branch Patterns** to prevent breaking changes on `main` or `release/*`.
- **Cleanup**: Delete or reset history for services and clients in the management tabs.
- **Tokens**: Users create their own API tokens at `/account.html`.

---

## Accessing the Dashboard

Open `/admin.html` in your browser and sign in with an admin account. On a fresh installation the only admin account is **root** — see [Getting Started](getting-started.md) for the initial password procedure.

![Admin login screen](images/Screenshot_20260421_230832.png)

After a successful login the dashboard loads with all management sections organized into tabs: **Observability**, **User Management**, **System Config**, and **Services & Clients**.

![Admin dashboard](images/Screenshot_20260421_230857.png)

## Observability

The **Observability** tab provides real-time insights into the system's health:

- **System Stats**: Live view of CPU usage, memory consumption, and uptime.
- **Request Counters**: Total requests and 5xx failure counters for business logic.
- **Log Viewer**: An in-memory ring buffer showing the last 100 log messages with level filtering.
- **Debug Tracing**: Toggleable flags to enable detailed tracing for business logic or admin activity at runtime.

## Developer Mode

Developer Mode is configured via the unified **Authentication** section of the admin dashboard. Selecting **Dev Mode** allows public API endpoints (`/provide`, `/require`, `/report`) to be accessible without any authentication. This is useful for local development and quick testing, but should not be used in production.

### Enabling Dev Mode (local only)

Because Dev Mode disables authentication entirely, it is protected by a **production safety gate** so a stray environment variable or a persisted setting left behind by configuration drift cannot silently open up a production instance. Dev Mode only becomes active when **both** of the following are true:

1. It is **requested** — either by selecting **Dev Mode** in the admin dashboard (persisted setting) or by setting `SANSHAIN_DEV_MODE=true` in the environment.
2. The safety gate `ALLOW_INSECURE_DEV_MODE=true` is set in the environment.

If Dev Mode is requested without the gate, the service **fails closed**: authentication stays enforced and the startup log emits a clear `SECURITY:` error explaining that Dev Mode was refused. Set `ALLOW_INSECURE_DEV_MODE=true` only on a trusted local machine — never in a production deployment or shared environment.

```bash
# Local development only — never set this gate in production:
ALLOW_INSECURE_DEV_MODE=true SANSHAIN_DEV_MODE=true cargo run
```

## Protected Branches

Protected branches enforce **immutable endpoint paths**. Once an endpoint is published on a protected branch, its schema cannot change; consumers can rely on it being stable. To evolve an endpoint you must bump the version in the path (e.g. `/api/v1/users` → `/api/v2/users`).

By default `main` and `master` are protected. You can add or remove patterns:

- Type a pattern into the text field (e.g. `release/*`) and press **Add**.
- Click the **×** button next to an existing pattern to remove it.

On non-protected (feature) branches, endpoint definitions can be freely overwritten.

## Held Provides

Outside onboarding, a Provide carrying a breaking change on a protected branch is **held for review**
rather than discarded. Refusal used to destroy the very thing you would need in order to overrule it:
the audit log recorded that something was rejected and why, but the submitted spec was gone.

The submission is now kept in full, with the reason it was refused and who pushed it. One entry per
Producer, branch and API type — a new refusal replaces the previous one, so CI pushing on every commit
files one problem rather than dozens, and what you review is at most one push old.

If the Producer fixes the problem itself and pushes something acceptable, the held entry is discarded.
That is what stops an approver later applying a stale submission over the top of the change that
resolved it.

**What the pushing Producer sees.** Still `409`. The spec is not live and no Consumer can resolve it,
so the build must stay red — a green build for a spec nobody can see would be worse than a failure.
The response body now identifies the held entry:

```json
{
  "error": "Breaking changes detected on protected branch 'master' of service 'orders': ...",
  "pending_id": 42,
  "status": "awaiting_review"
}
```

Holding is audited as `QUARANTINED_SPEC`, under the existing **Rejected (Blocked)** timeline type.

### Reviewing them

The **Held Provides** section of the admin dashboard lists what is waiting, with the reason, who
pushed it, and the submitted spec itself — so you decide with the content in front of you rather than
from a one-line reason.

- **Accept** applies the held spec. The stale-base-version check is skipped: it exists to stop a
  Producer overwriting work it had not seen, and an approver applying a spec they have just read is a
  different act. Everything else happens as usual — version bump, soft deletes, version history.
- **Reject** discards the entry. The Producer's next Provide is evaluated fresh.

Both write an audit entry naming who decided. An administrator may act on any Producer; a maintainer
only on the Producers they maintain, and the list is filtered accordingly rather than refusing to open.

There is deliberately no second-pair-of-eyes rule: a maintainer may approve a change they pushed
themselves.

| Method   | Endpoint                             | Description                                |
|----------|--------------------------------------|--------------------------------------------|
| `GET`    | `/admin/pending-specs`               | Held Provides you may act on, with a count.|
| `GET`    | `/admin/pending-specs/{id}`          | One held Provide, including its spec.      |
| `POST`   | `/admin/pending-specs/{id}/accept`   | Apply it.                                  |
| `DELETE` | `/admin/pending-specs/{id}`          | Discard it.                                |

## Producer Onboarding

A Producer whose API is not yet stable can be put into **onboarding**. While it is on, a Provide to a
protected branch is not gatekept: none of the five refusals apply — the OpenAPI compatibility check,
the AsyncAPI and proto checks, the refusal to remove a non-deprecated endpoint, and the refusal to
re-introduce a removed one.

It applies unconditionally: the Producer does not pass a `force` flag. These pushes come from CI with
no special handling, and requiring a flag would leave the problem where it started.

**What onboarding does not do.** The branch stays protected in every sense that preserves data.
Deletes remain soft and version history is still written — so onboarding is strictly less destructive
than the workaround it replaces, deleting the branch by hand, which throws that history away.

**Versions keep telling the truth.** A breaking change bumps the major version as normal. That is
deliberate: the flag has no expiry, so the major number is the signal of who is still thrashing and
who has settled. A Producer sitting at `14.0.0` is visibly not ready; one that has held `2.x` for a
month is ready to have onboarding switched off.

Every breaking change let through writes an `ACCEPTED_BREAKING` audit entry carrying the reason the
refusal would have given — so instead of a stream of failures you get a reviewable record of exactly
what each Producer broke.

> Onboarding **never expires**. Nothing will remind you that it is still on, so
> `GET /admin/producers/onboarding` lists which Producers are currently lenient. Check it before you
> rely on protection.

An administrator, or a maintainer of that Producer, may turn it on and off.

| Method | Endpoint                              | Description                                          |
|--------|---------------------------------------|------------------------------------------------------|
| `GET`  | `/admin/producers/onboarding`         | Producers currently in onboarding.                   |
| `GET`  | `/admin/producers/{name}/onboarding`  | Whether one Producer is in onboarding.               |
| `PUT`  | `/admin/producers/{name}/onboarding`  | Set it. Body: `{"onboarding": true}`.                |

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
| `maintainer`   | Producer administration, onboarding and pending-spec review — **scoped**.    |

`maintainer` cannot be granted instance-wide: it is meaningless without the set of Producers it is
over, so it is assigned as a scope rather than granted as a role.

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
is refused on everybody else's. This covers branch administration, onboarding, held-spec review — and
editing the Producer's endpoints through the spec editor.

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

## Service Management

The **Services** section lists all services that have provided at least one OpenAPI specification. Each entry shows the service name.

You can expand any service using the ▶ button to view its active branches. Each branch supports the following actions:

- **Reset History** — prunes all old, inactive endpoint versions for the selected branch, renumbers the latest active version of each endpoint to `1`, and resets the branch's version counter. Existing endpoints and client dependencies are fully preserved, ensuring zero disruption for active clients.
- **Delete** — removes the selected branch and all its associated endpoints.

At the service level, you can perform:

- **Delete** — removes the service and **cascades** to all its branches, endpoints, and related client dependencies.

A confirmation dialog appears before executing any deletion or reset operation.

For a detailed view of a service's branches and endpoints, use the [Service Overview](/service.html) page instead.

## Client Management

The **Clients** section lists all clients that have registered at least one dependency via `/require`.

- **Delete** — removes the client and all its recorded dependencies. A confirmation dialog appears before deletion.

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
