# Getting Started

This guide walks you through installing Sanshain Service, logging in for the first time, and configuring the essential settings.

## Installation with Docker (SQLite — default)

The quickest way to get Sanshain running is to pull the pre-built image from the GitHub Container Registry:

```bash
docker pull ghcr.io/paxel/sanshain-service:latest
docker run -p 3000:3000 -v ~/sanshain-db-dir/:/data ghcr.io/paxel/sanshain-service:latest
```

This creates a fresh SQLite database with secure defaults and generates a **root** account with a random password.

> **Tip:** The database is persisted in the mounted volume (`~/sanshain-db-dir/`), so your data survives container restarts.

## Installation with Docker (PostgreSQL)

To use PostgreSQL instead of SQLite, set the `DATABASE_URL` environment variable to a PostgreSQL connection string. Sanshain auto-detects the backend from the URL prefix (`postgres://` or `postgresql://`).

### 1. Start a PostgreSQL instance

If you don't already have a PostgreSQL server, you can start one with Docker:

```bash
docker network create sanshain-net

docker run -d --name sanshain-postgres --network sanshain-net \
  -e POSTGRES_USER=sanshain \
  -e POSTGRES_PASSWORD=changeme \
  -e POSTGRES_DB=sanshain \
  -v sanshain-pgdata:/var/lib/postgresql/data \
  postgres:16
```

### 2. Start Sanshain with PostgreSQL

```bash
docker run -p 3000:3000 --network sanshain-net \
  -e DATABASE_URL=postgres://sanshain:changeme@sanshain-postgres:5432/sanshain \
  ghcr.io/paxel/sanshain-service:latest
```

Sanshain runs all required migrations automatically on startup — no manual schema setup is needed.

### Docker Compose example (PostgreSQL)

```yaml
services:
  sanshain:
    image: ghcr.io/paxel/sanshain-service:latest
    ports:
      - "3000:3000"
    environment:
      - DATABASE_URL=postgres://sanshain:changeme@postgres:5432/sanshain
      - RUST_LOG=sanshain_service=info,tower_http=info
    depends_on:
      postgres:
        condition: service_healthy
    restart: unless-stopped

  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: sanshain
      POSTGRES_PASSWORD: changeme
      POSTGRES_DB: sanshain
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U sanshain"]
      interval: 5s
      timeout: 5s
      retries: 5
    restart: unless-stopped

volumes:
  pgdata:
```

> **Production tip:** Replace `changeme` with a strong password and consider using Docker secrets or an external secret manager.

## Running without Docker

### SQLite (default)

```bash
cargo build --release
./target/release/sanshain_service
```

The SQLite database file `sanshain.db` is created automatically in the working directory.

### PostgreSQL

Ensure you have a running PostgreSQL server with a database created, then set `DATABASE_URL`:

```bash
# Create the database (if it doesn't exist yet)
createdb -U postgres sanshain

# Start Sanshain
DATABASE_URL=postgres://postgres:yourpassword@localhost:5432/sanshain \
  ./target/release/sanshain_service_bin
```

Migrations are applied automatically on startup.

## First Login

On first start, Sanshain prints the initial root credentials to the console output:

![Initial root credentials printed to the console](images/Screenshot_20260314_075130.png)

Copy the password, then open the URL shown in the log message to access the admin dashboard. Log in with username **root** and the generated password.

![Admin dashboard after login](images/Screenshot_20260314_075115.png)

After a successful login, you will see the admin dashboard:

## Change the Root Password

Immediately change the root password to something memorable — or store the generated one in a password manager.

![Password change dialog](images/Screenshot_20260314_075210.png)

## Initial Configuration

By default, Sanshain is **completely locked down**: all API endpoints require authentication, and user self-registration is disabled. Before the service is usable you need to adjust at least one of the following settings from the admin dashboard:

| Setting | What it does |
|---|---|
| **Developer Mode** | Opens all non-admin REST endpoints (`/provide`, `/require`, `/report`) without authentication. Useful for local development and quick testing. |
| **Local User Registration** | Allows users to create accounts via the web UI. New accounts still require admin approval before they can log in. |

You can also manage **protected branch patterns** from the admin dashboard. By default, `main` and `master` are protected, meaning endpoint definitions on those branches are immutable — any change to an existing endpoint path requires a version bump (e.g., `/api/v1/users` → `/api/v2/users`).

## Next Steps

- **Populate sample data** — Run the [demo script](../demo.sh) to register example services and clients so you can explore the UI right away. See the [Demo Script](../README.md#demo-script) section in the README for details.
- **Provide your first spec** — Use `POST /provide` to upload an OpenAPI YAML for one of your services. See the [API Usage](../README.md#api-usage) section.
- **Create an API token** — Visit `/account.html` to generate a `san_`-prefixed token for CI pipelines.
- **Explore the dependency graph** — Open `/service.html` to browse services, branches, endpoints, and client dependencies.
