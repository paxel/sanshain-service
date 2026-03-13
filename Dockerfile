# Stage 1: Build
FROM rust:1.85-alpine AS builder

RUN apk add --no-cache musl-dev sqlite-dev openssl-dev pkgconfig

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY static/ static/
COPY templates/ templates/
#COPY tests/ tests/

RUN cargo build --release

# Stage 2: Runtime
FROM alpine:3.21

RUN apk add --no-cache sqlite-libs

COPY --from=builder /app/target/release/sanshain_service_bin /usr/local/bin/sanshain
COPY static/ /app/static/

WORKDIR /app

ENV DATABASE_URL="sqlite:/data/sanshain.db?mode=rwc"

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://localhost:3000/health || exit 1

CMD ["sanshain"]
