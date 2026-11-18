#!/usr/bin/env bash
# Runs the command it is given once the Postgres image of the tests is on the machine:
#   scripts/ci-test.sh cargo test --locked --all-features
# The shared Rust gate runs the tests and the coverage measurement under it (ci.yml,
# test-wrapper). The Postgres tests start their database through testcontainers, which pulls
# the image on demand; several test binaries pulling it at the same moment fail now and then
# ("bytes remaining on stream"), so it is pulled once before they start. The tag is
# POSTGRES_IMAGE in tests/common/mod.rs. A pull that fails three times is left to the retries
# of the tests themselves.
set -euo pipefail

image="postgres:11-alpine"

for _ in 1 2 3; do
  if docker pull "$image"; then
    break
  fi
  sleep 5
done

exec "$@"
