# Stage 1: Build
FROM rust:1.99.1-alpine AS builder

# SQLite is compiled into the binary (sqlx sqlite-bundled): no sqlite package here or below.
RUN apk add --no-cache musl-dev openssl-dev pkgconfig

WORKDIR /app
COPY Cargo.toml Cargo.lock LICENSE ./
COPY src/ src/
COPY static/ static/
COPY templates/ templates/
COPY benches/ benches/
#COPY tests/ tests/

RUN cargo build --release --locked

# Stage 2: Runtime
FROM alpine:3.24

# ca-certificates: the platform trust store, so ldaps:// against a publicly
# signed directory works without a certificate mount. Private CAs still go
# through EXTRA_CA_CERTS_DIR.
# Upgrade the base packages first: Alpine ships OpenSSL and zlib fixes between its
# point releases, and the weekly grype scan fails on a released image that misses one.
RUN apk upgrade --no-cache && apk add --no-cache tini ca-certificates

COPY --from=builder /app/target/release/sanshain_service /usr/local/bin/sanshain
COPY static/ /app/static/
COPY src/infrastructure/migrations/ /app/src/infrastructure/migrations/

WORKDIR /app

# The service runs as sanshain (UID/GID 1000, as in the Kubernetes manifests),
# not root. /data belongs to it, so a new volume mounted there starts writable.
RUN addgroup -S -g 1000 sanshain \
    && adduser -S -D -H -u 1000 -G sanshain -h /app sanshain \
    && mkdir -p /data \
    && chown sanshain:sanshain /data

ENV DATABASE_URL="sqlite:/data/sanshain.db?mode=rwc"

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://127.0.0.1:3000/health || exit 1

USER 1000:1000
ENTRYPOINT ["tini", "--"]
CMD ["sanshain"]
