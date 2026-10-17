# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.6.0]

### Added
- Webhooks: Sanshain emits signed events (`version.published`, `provide.rejected`, `version.deprecated`, `dependency.recorded`, `branch.cut`, `catalog.recorded`) as a `POST` to registered hooks — global hooks with event-type and Producer-glob filters on the admin page, a Producer's or Consumer's own hooks on its page (a Consumer's hooks follow its require graph). The body is the JSON envelope or a ready-made Slack or Teams message, signed with HMAC-SHA256 (`Sanshain-Signature`) under a secret shown once; events are written to an outbox with the fact they describe and delivered by a worker with exponential backoff for 24 hours, with a delivery log, pause/resume, redeliver, test events and secret rotation. Receivers must be on an admin-managed allow-list of hosts (`/admin/settings/webhook-allowed-hosts`); `WEBHOOKS_ENABLED`, `WEBHOOK_WORKER_INTERVAL_SECS`, `WEBHOOK_DELIVERY_TIMEOUT_SECS` and `PUBLIC_BASE_URL` configure it, `sanshain_webhook_deliveries_total` and `sanshain_webhook_outbox_pending` watch it. See [CI Integration — Webhooks](docs/ci-integration.md#webhooks) and ADR-0009.
- The catalog push also records the system's owner, description, source location and links from its Backstage `catalog-info.yaml`; the graph's node panel, the Producer and Consumer pages (`GET /admin/producers/{name}`, `GET /admin/consumers/{name}`), `/report` (`systems`), the graph scope payload and the MCP `describe_service` tool show them, and a push with unchanged content changes nothing.
- On SQLite, admins can download a consistent snapshot of the database while the service runs (`GET /admin/backup/sqlite`, also a button on the admin page), the Helm chart renders an optional backup CronJob for SQLite or `pg_dump`, and the deployment guide documents backup and restore for both engines.
- Every release ships CycloneDX SBOMs (as release assets and as a cosign attestation on the image), the image is signed keyless with cosign, and the weekly security run scans the released image with grype; the deployment guide explains how to verify a release.

### Changed
- Every in-memory data cache now expires its entries after 10 s, so on a multi-replica PostgreSQL install a provide handled by one replica is visible on the others within 10 s instead of until memory pressure evicts the old entry; a single replica behaves as before. See the Scaling section of the deployment guide for the session-affinity recommendation.
- The container image now trusts public certificate authorities, so `ldaps://` against a publicly signed directory works without a certificate mount; private CAs still go through `EXTRA_CA_CERTS_DIR`.

### Fixed
- An LDAP directory that accepts the connection and never answers (typically `ldaps://` on the plain port or StartTLS on the TLS port) now fails within 10 s with a message that names the URL and the likely port/scheme mismatch, instead of hanging until the ingress answers "Bad Gateway"; `LDAP_OPERATION_TIMEOUT_SECS` overrides the limit.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
