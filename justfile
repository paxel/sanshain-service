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

# Run Playwright UI smoke tests (requires service running)
ui-test:
    npx playwright test tests/ui/smoke.test.js
