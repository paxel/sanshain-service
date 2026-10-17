# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.6.0]

### Added
- On SQLite, admins can download a consistent snapshot of the database while the service runs (`GET /admin/backup/sqlite`, also a button on the admin page), the Helm chart renders an optional backup CronJob for SQLite or `pg_dump`, and the deployment guide documents backup and restore for both engines.

### Changed
- Every in-memory data cache now expires its entries after 10 s, so on a multi-replica PostgreSQL install a provide handled by one replica is visible on the others within 10 s instead of until memory pressure evicts the old entry; a single replica behaves as before. See the Scaling section of the deployment guide for the session-affinity recommendation.
- The container image now trusts public certificate authorities, so `ldaps://` against a publicly signed directory works without a certificate mount; private CAs still go through `EXTRA_CA_CERTS_DIR`.

### Fixed
- An LDAP directory that accepts the connection and never answers (typically `ldaps://` on the plain port or StartTLS on the TLS port) now fails within 10 s with a message that names the URL and the likely port/scheme mismatch, instead of hanging until the ingress answers "Bad Gateway"; `LDAP_OPERATION_TIMEOUT_SECS` overrides the limit.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
