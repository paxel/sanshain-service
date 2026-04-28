# Sanshain Service Tasks

# Run all local checks (fmt, lint, test)
check:
    cargo check-all

# Check formatting
fmt-check:
    cargo fmt-check

# Fix formatting
fmt:
    cargo fmt

# Run clippy
lint:
    cargo lint

# Run unit and integration tests
test:
    cargo test

# Run the automated bash integration test suite (requires service running)
itest:
    cargo itest

# Run Playwright UI smoke tests (requires service running)
ui-test:
    cargo ui-test
