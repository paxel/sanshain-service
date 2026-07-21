# Configuration

Sanshain Service can be configured using environment variables.

| Variable                       | Default                                   | Description                                                                                               |
|--------------------------------|-------------------------------------------|-----------------------------------------------------------------------------------------------------------|
| `DATABASE_URL`                 | `sqlite:sanshain.db?mode=rwc`             | Database connection string. Use `postgres://user:pass@host:5432/dbname` for PostgreSQL.                   |
| `BIND_ADDRESS`                 | `0.0.0.0:3000`                            | Address and port to listen on.                                                                            |
| `MAX_POSTGRES_CONNECTIONS`     | `20`                                      | Max connection pool size for PostgreSQL.                                                                  |
| `MAX_SQLITE_CONNECTIONS`       | `1`                                       | Max connection pool size for SQLite.                                                                      |
| `SQLITE_BUSY_TIMEOUT_MS`       | `5000`                                    | SQLite busy timeout in milliseconds.                                                                      |
| `CLEANUP_INTERVAL_SECS`        | `3600`                                    | Interval for background cleanup tasks (branches, dependencies).                                           |
| `CSRF_MAX_AGE_HOURS`           | `24`                                      | Maximum age of CSRF tokens before they are pruned.                                                        |
| `LOG_BUFFER_SIZE`              | `100`                                     | Number of messages kept in the in-memory log buffer per level.                                            |
| `SPEC_UPDATED_CHANNEL_SIZE`    | `100`                                     | Size of the broadcast channel for specification updates.                                                  |
| `MAX_SPEC_BODY_BYTES`          | `4194304` (4 MiB)                         | Maximum HTTP request body size in bytes. Requests with a larger body are rejected with `413`.             |
| `CAPTURE_LOG_FILTER`           | `sanshain_service=debug,tower_http=debug` | Log level filter for the in-memory log capture buffer.                                                    |
| `PROMETHEUS_ENDPOINT`          | `/metrics`                                | Path for Prometheus metrics.                                                                              |
| `STATIC_DIR`                   | `static`                                  | Directory containing static web assets.                                                                   |
| `LOGIN_SESSION_DURATION_HOURS` | `24`                                      | Duration of user login sessions in hours.                                                                 |
| `INITIAL_ADMIN_USERNAME`       | `root`                                    | Username for the initial admin account.                                                                   |
| `INITIAL_ADMIN_PASSWORD`       | *random*                                  | Pre-defined password for the initial admin account.                                                       |
| `SANSHAIN_DEV_MODE`            | `false`                                   | Requests dev mode (unauthenticated API). Only active when `ALLOW_INSECURE_DEV_MODE=true`. Local only.     |
| `ALLOW_INSECURE_DEV_MODE`      | `false`                                   | Safety gate: dev mode only activates when this is `true`; fails closed otherwise. Not for production.     |
| `INSTANCE_ID`                  | *random UUID*                             | Unique ID for this service instance.                                                                      |
| `CACHE_MEMORY_MB`              | `256`                                     | In-memory cache size in MB. Set to `0` to disable caching entirely. Configurable at runtime via admin UI. |
| `LOG_FORMAT`                   | `text`                                    | Log output format (`text` or `json`).                                                                     |
| `OTEL_EXPORTER_OTLP_ENDPOINT`  | `http://localhost:4317`                   | OTLP/gRPC collector endpoint for distributed tracing.                                                     |
| `RUST_LOG`                     | `sanshain_service=info,tower_http=info`   | Log level filter (e.g., `sanshain_service=debug,tower_http=debug` for verbose output).                    |
