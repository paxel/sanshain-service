# Sanshain Service Tasks

# Run all local checks (fmt, lint, test)
check: fmt-check lint test

# Check formatting
fmt-check:
    cargo fmt --check

# Fix formatting
fmt:
    cargo fmt

# Run clippy
lint:
    cargo clippy -- -D warnings

# Run unit and integration tests
test:
    cargo test

# Run the automated bash integration test suite (requires service running)
itest:
    ./scripts/itest.sh

# Run Playwright UI smoke tests + de-inlining safety nets (requires service running)
ui-test:
    npx playwright test tests/ui/smoke.test.js tests/ui/no_console_errors.test.js tests/ui/de_inlined.test.js tests/ui/permissions.test.js tests/ui/structure.test.js tests/ui/endpoint_filter.test.js

# Regenerate the README/docs screenshots (site/assets/images/*.png). Builds a release
# binary, seeds demo data, drives Playwright against a real instance, and
# overwrites the images in place — nothing else is touched.
screenshots:
    ./scripts/update_screenshots.sh

# Re-record the documentation walkthroughs (site/evidence/**, the chapter screenshots they
# share). Builds a release binary, starts a fresh instance on a free port with a temporary
# SQLite file, replays the Developer and the Operator story with the real clients and curl,
# fails on any answer that differs from what a step expects, and overwrites the evidence.
record-walkthroughs:
    ./scripts/record-walkthroughs.sh

# Build the documentation site (site/ -> site-dist/)
site:
    node scripts/build-site.js

# Build every released version of the documentation site plus the working tree
# (site-dist/, latest at the root, each release under vX.Y.Z/), as Pages publishes it
site-all:
    node scripts/build-site-all.js

# Build the documentation site and preview it at http://localhost:8080/
site-serve:
    node scripts/build-site.js --serve 8080
