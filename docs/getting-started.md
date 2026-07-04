# Getting Started

This guide walks you through installing Sanshain Service, logging in for the first time, and configuring the essential settings.

### TL;DR
1. **Run**: `docker run -p 3000:3000 -v ~/sanshain-data:/data ghcr.io/paxel/sanshain-service:latest`
2. **Login**: Go to `http://localhost:3000/admin.html` with the password from the logs.
3. **Configure**: For a quick unauthenticated local trial, start the container with `-e ALLOW_INSECURE_DEV_MODE=true -e SANSHAIN_DEV_MODE=true` (local only — never in production), or create an API token in Account settings for authenticated access.

---

## Installation with Docker

### SQLite (Default)
The quickest way to get Sanshain running. The database is persisted in the mounted volume.

```bash
docker run -p 3000:3000 -v ~/sanshain-db-dir/:/data ghcr.io/paxel/sanshain-service:latest
```

### PostgreSQL
Set the `DATABASE_URL` environment variable. Sanshain auto-detects the backend.

```bash
docker run -p 3000:3000 \
  -e DATABASE_URL=postgres://user:pass@host:5432/db \
  ghcr.io/paxel/sanshain-service:latest
```

#### Docker Compose Example (PostgreSQL)
See [deploy/docker-compose.release.template.yaml](../docker-compose.release.template.yaml) for a full example.

---

## Running from Source

1. **Build**: `cargo build --release`
2. **Run**: `DATABASE_URL=sqlite://sanshain.db ./target/release/sanshain_service`

Migrations are applied automatically on startup.

---

## First Login & Setup

1. **Credentials**: Check the console output (stderr) for the initial root password.
2. **Access**: Open `http://localhost:3000/admin.html`.
3. **Password**: Change the `root` password immediately.
4. **Permissions**: By default, Sanshain is locked. You must:
   - **Enable Developer Mode**: Allows unauthenticated API access (useful for local testing). For safety this only activates when the service is also started with the `ALLOW_INSECURE_DEV_MODE=true` environment variable — see [Developer Mode](administration.md#enabling-dev-mode-local-only). Never enable it in production.
   - **Approved Users**: New users must be approved by an admin before logging in.

---

## Next Steps
- [**User Guide**](user-guide.md) — How to provide and require APIs.
- [**API Usage**](api-usage.md) — Detailed endpoint reference.
- [**Populate Demo Data**](../README.md#demo-script) — Use the `./demo.sh` script to see Sanshain in action.
