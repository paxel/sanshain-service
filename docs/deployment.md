# Deployment Guide

This guide covers production-ready deployment of Sanshain Service on Kubernetes.

## Kubernetes (via Kustomize)

The `deploy/kubernetes` directory contains base manifests suitable for customization via Kustomize.

### Prerequisites

- A Kubernetes cluster.
- `kubectl` installed and configured.

### Configuration

1. **Namespace**: It's recommended to create a dedicated namespace.
   ```bash
   kubectl create namespace sanshain
   ```

2. **Secrets**: Update `deploy/kubernetes/secret.yaml` with your base64-encoded values for `DATABASE_URL` and `INITIAL_ADMIN_PASSWORD`.
   ```bash
   echo -n "sqlite:///data/sanshain.db" | base64
   echo -n "your-secure-password" | base64
   ```

3. **Ingress**: Update `deploy/kubernetes/ingress.yaml` with your actual hostnames and TLS secret name.

### Deployment

To apply the manifests using Kustomize:

```bash
kubectl apply -k deploy/kubernetes -n sanshain
```

This will create the following resources:
- `ConfigMap`: Application settings.
- `Secret`: Sensitive credentials.
- `PersistentVolumeClaim`: 1Gi storage for SQLite (mounted at `/data`).
- `Deployment`: 1 replica of the Sanshain Service (see [Scaling](#scaling)).
- `Service`: ClusterIP service for internal access.
- `Ingress`: External access with TLS support.

---

## Helm Chart

For more complex deployments or automated distribution, a Helm chart is provided in `deploy/helm/sanshain`.

### Prerequisites

- [Helm](https://helm.sh/docs/intro/install/) v3+.

### Installation

1. **Install with default settings (SQLite)**:
   ```bash
   helm install sanshain deploy/helm/sanshain -n sanshain --create-namespace
   ```

2. **Install with external PostgreSQL**:
   Create a `my-values.yaml` file:
   ```yaml
   persistence:
     enabled: false # No local PVC needed for SQLite
   
   secrets:
     databaseUrl: "postgres://user:password@postgres-host:5432/sanshain"
   ```
   Then install:
   ```bash
   helm install sanshain deploy/helm/sanshain -f my-values.yaml -n sanshain
   ```

### Common Configuration Options

See `deploy/helm/sanshain/values.yaml` for a complete list of parameters. Key options include:

| Parameter                      | Description                                                  | Default              |
|--------------------------------|--------------------------------------------------------------|----------------------|
| `replicaCount`                 | Number of pods to run                                        | `1`                  |
| `image.tag`                    | Image tag to deploy                                          | (Chart `appVersion`) |
| `ingress.enabled`              | Enable Ingress resource                                      | `false`              |
| `persistence.enabled`          | Enable persistent storage for SQLite                         | `true`               |
| `persistence.size`             | Size of the persistent volume                                | `1Gi`                |
| `config.initialAdminUsername`  | Initial admin username                                       | `root`               |
| `secrets.initialAdminPassword` | Initial admin password (if empty, random password is logged) | `""`                 |

### Uninstallation

```bash
helm uninstall sanshain -n sanshain
```

## Persistence Notes

### SQLite
When using SQLite (default), the database file is stored at `/data/sanshain.db`. The `persistence.enabled` setting must be `true` to ensure data survives pod restarts.

### PostgreSQL
If using PostgreSQL, set `persistence.enabled` to `false` (unless you need persistent storage for other purposes) and provide the `DATABASE_URL` in `secrets.databaseUrl`.

## Scaling

The default deployment runs a single replica because the default SQLite backend
stores its database on a `ReadWriteOnce` PVC that only one pod can mount, and
concurrent writers would corrupt the SQLite file.

To run more than one replica, switch to PostgreSQL (set `DATABASE_URL` to a
`postgres://...` URL and disable the SQLite PVC), then raise `replicas`
(Kustomize) or `replicaCount` (Helm).

Each replica keeps its own in-memory caches (see `CACHE_MEMORY_MB` in
[Configuration](configuration.md)). A write invalidates the caches of the
replica that handled it immediately; every other replica picks it up within
**10 seconds**, when its cached entries expire. Consequently a build that
provides a Version on one replica and requires it on another within the same
10 seconds can still be answered with "version not found", the previous
Snapshot content or the previous report.

To close that window entirely, enable session affinity (sticky sessions) on the
ingress for the API routes (`/provide*`, `/require*`, `/require-bundle`,
`/catalog/*`, `/producers/*`), so one build's requests land on one replica.
Otherwise, have the pipeline wait 10 seconds between the provide and the first
require.

## Backup and restore

Sanshain keeps everything in one database, so a backup is a copy of that
database and a restore is putting the copy back. How the copy is taken depends
on the engine.

### SQLite

Do not copy `sanshain.db` from the volume while the service runs: with the
write-ahead log open, a file copy can be inconsistent. Instead, Sanshain
produces the copy itself:

```
GET /admin/backup/sqlite
Authorization: Bearer <token>
```

answers a consistent snapshot made by the engine (`VACUUM INTO`) while the
service keeps running, streamed as `application/vnd.sqlite3` and named
`sanshain-<instance version>-<UTC timestamp>.db`. The route requires the
`run_destructive_operations` permission (admins and root hold it); a `full`
API token of such an account is accepted, a `read_only` token is refused with
`403`. Only one snapshot runs at a time — a second request gets `409`. Every
download is recorded in the audit log with the Actor and the size. The same
snapshot is available as **Download database snapshot** on the admin page
(Database Configuration), on SQLite only. On PostgreSQL the route answers `409`
and names `pg_dump`.

**Scheduled (Helm).** The chart renders a CronJob that fetches the snapshot to a
backup volume and keeps the newest `retain` files:

```yaml
backup:
  enabled: true
  schedule: "0 2 * * *"
  retain: 7
  # existingClaim: nightly-backups   # otherwise the chart creates a 5Gi claim
  tokenSecret:
    name: sanshain-backup-token      # kubectl create secret generic sanshain-backup-token --from-literal=token=<api token>
    key: token
```

Mint the token on `/account.html` as an admin with the **full** scope; it is
the only credential the job holds. Kustomize users find the same job as a
commented example in `deploy/kubernetes/backup-cronjob-sqlite.yaml`.

**Restore.**

1. Scale Sanshain to zero (`kubectl scale deployment sanshain --replicas=0`).
2. On the data volume replace `sanshain.db` with the snapshot and delete
   `sanshain.db-wal` and `sanshain.db-shm` if present.
3. Scale back up. The migration runner brings the snapshot's schema up to the
   running version, so a snapshot from an older instance version restores fine;
   a snapshot from a *newer* version does not.

### PostgreSQL

Back up with `pg_dump`, as for any PostgreSQL database:

```bash
pg_dump --format=custom --file=sanshain.dump "$DATABASE_URL"
```

**Scheduled (Helm).** The same `backup` section renders a `pg_dump` CronJob
instead when `backup.engine` is `postgres` or `secrets.databaseUrl` is a
`postgres://` URL; it reads `DATABASE_URL` from the chart's own Secret, so no
further credential is needed:

```yaml
backup:
  enabled: true
  schedule: "0 2 * * *"
  retain: 7
```

Kustomize: `deploy/kubernetes/backup-cronjob-postgres.yaml`.

**Restore.**

1. Scale Sanshain to zero.
2. `pg_restore --clean --if-exists --dbname="$DATABASE_URL" sanshain.dump`
3. Scale back up; migrations run on start as above.

| Parameter              | Description                                                         | Default       |
|------------------------|---------------------------------------------------------------------|---------------|
| `backup.enabled`       | Render the backup CronJob                                           | `false`       |
| `backup.schedule`      | Cron schedule                                                       | `"0 2 * * *"` |
| `backup.engine`        | `sqlite` or `postgres`; empty derives it from `secrets.databaseUrl` | `""`          |
| `backup.retain`        | Backup files kept on the volume                                     | `7`           |
| `backup.existingClaim` | Mount this claim instead of creating one                            | `""`          |
| `backup.storage.size`  | Size of the claim the chart creates                                 | `5Gi`         |
| `backup.tokenSecret`   | SQLite: Secret `name` and `key` holding the API token               | `key: token`  |

## Health and readiness probes

- **Liveness** (`GET /health`): returns `OK` without touching the database, so a
  transient database outage does not cause the pod to be restarted.
- **Readiness** (`GET /ready`): runs a trivial `SELECT 1` against the database
  and returns `200` when it succeeds or `503` when the database is unreachable.
  Kubernetes stops routing traffic to a pod whose database is down until it
  recovers.

## Security

- **Non-Root**: The container is configured to run as a non-root user (UID 1000).
- **Security Context**: Standard Pod and Container security contexts are applied in both Kustomize and Helm templates.
- **TLS**: Ingress templates support TLS termination.
- **Outbound TLS trust**: The image ships the system CA bundle, so `ldaps://` against a directory
  with a publicly trusted certificate works out of the box. A private or corporate CA still goes
  through the `EXTRA_CA_CERTS_DIR` mount, exactly as before — see
  [Additional CA certificates](configuration.md#additional-ca-certificates).

## Verifying a release

Every release carries three supply-chain artifacts, all produced by the release workflow of
this repository (`.github/workflows/release.yml`):

- The image `ghcr.io/paxel/sanshain-service:<tag>` is signed keyless with
  [cosign](https://docs.sigstore.dev/cosign/) through the workflow's GitHub OIDC identity on
  the public Sigstore instance. No key to distribute: the certificate names the workflow.
- A CycloneDX SBOM of the image is attached to the image as a cosign attestation and to the
  GitHub release as the asset `sanshain-service-<tag>.cdx.json`.
- A CycloneDX SBOM of the dependencies compiled into the binaries (from `Cargo.lock` and
  `package-lock.json`) is on the GitHub release as `sanshain-service-<tag>-source.cdx.json`.

Both SBOM assets are listed under **Assets** on the release page
(`https://github.com/paxel/sanshain-service/releases/tag/<tag>`).

Verify the signature with `cosign` (replace `v2.6.0` with the tag or the `@sha256:…` digest
you deploy):

```sh
cosign verify ghcr.io/paxel/sanshain-service:v2.6.0 \
  --certificate-identity-regexp '^https://github.com/paxel/sanshain-service/.github/workflows/release.yml@refs/tags/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Verify the SBOM attestation the same way; the attested SBOM is the `predicate` of the printed
in-toto statement, so this also extracts it without the GitHub release:

```sh
cosign verify-attestation --type cyclonedx ghcr.io/paxel/sanshain-service:v2.6.0 \
  --certificate-identity-regexp '^https://github.com/paxel/sanshain-service/.github/workflows/release.yml@refs/tags/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  | jq -r .payload | base64 -d | jq .predicate > sanshain-service.cdx.json
```

A signature or attestation whose certificate names any other identity or issuer fails
verification, so an admission policy can require exactly this identity. Only a release built from
a pushed tag (the normal release path) verifies with this command: a release started by hand
through `workflow_dispatch` is signed with the branch it ran from (`@refs/heads/<branch>`) instead.

The weekly security workflow also scans the latest released image with grype; it fails only on
high or critical findings that have a fix and attaches the full report, unfixed findings
included, as a workflow artifact.
