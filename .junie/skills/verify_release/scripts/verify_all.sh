#!/usr/bin/env bash

# verify_all.sh — Comprehensive verification suite for Sanshain Service
# Orchestrates all Rust and JS tests, lints, and security audits.

set -euo pipefail

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log() { echo -e "${BLUE}[VERIFY]${NC} $1"; }
success() { echo -e "${GREEN}[PASS]${NC} $1"; }
failure() { echo -e "${RED}[FAIL]${NC} $1"; exit 1; }

# ---------------------------------------------------------------------------
# 1. Rust Quality Checks
# ---------------------------------------------------------------------------

log "Checking Rust formatting (cargo fmt)..."
cargo fmt --check || failure "Rust formatting check failed. Run 'cargo fmt' to fix."
success "Rust formatting is correct."

log "Running Rust Clippy (linting)..."
cargo clippy -- -D warnings || failure "Clippy found issues. Please fix them."
success "Clippy passed with zero warnings."

log "Running Cargo Audit (vulnerability check)..."
cargo audit || failure "Cargo audit found vulnerabilities."
success "Cargo audit passed."

log "Running Cargo Geiger (unsafe code check)..."
# Temporary workaround for cargo-geiger looking for non-existent compat_test.rs
touch tests/compat_test.rs
cargo geiger --brief || log "Warning: Cargo geiger reported some issues."
rm tests/compat_test.rs
success "Cargo geiger finished."

log "Running Rust unit tests..."
cargo test --lib || failure "Rust unit tests failed."
success "Rust unit tests passed."

# ---------------------------------------------------------------------------
# 2. JavaScript Quality Checks
# ---------------------------------------------------------------------------

log "Running ESLint for JavaScript..."
npx eslint static/js/ || failure "ESLint found issues in static/js/."
success "ESLint passed."

log "Checking Prettier for JavaScript..."
npx prettier --check static/js/ || failure "Prettier check failed for static/js/. Run 'npx prettier --write static/js/' to fix."
success "Prettier check passed."

# ---------------------------------------------------------------------------
# 3. Integration Tests (Requires running service)
# ---------------------------------------------------------------------------

log "Preparing for integration tests..."

# Use a temporary database and a different port to avoid conflicts
export DATABASE_URL="sqlite:verify_test.db?mode=rwc"
export INITIAL_ADMIN_PASSWORD="root_password" # Must match hardcoded password in Playwright smoke tests
export BIND_ADDRESS="127.0.0.1:3001"
export BASE_URL="http://127.0.0.1:3001" # For Playwright tests
export LOG_FORMAT="json" # Better for capturing logs if needed

# Clean up any existing test DB
rm -f verify_test.db verify_test.db-shm verify_test.db-wal

log "Starting service in background on $BIND_ADDRESS..."
cargo run > verify_service.log 2>&1 &
SERVICE_PID=$!

# Ensure service is killed on exit, even if script fails
cleanup() {
    log "Cleaning up..."
    if [ -n "${SERVICE_PID:-}" ]; then
        kill "$SERVICE_PID" 2>/dev/null || true
    fi
    rm -f verify_test.db verify_test.db-shm verify_test.db-wal
}
trap cleanup EXIT

# Wait for service to be ready
log "Waiting for service to be ready..."
MAX_RETRIES=60
RETRY_COUNT=0
while ! curl -s http://127.0.0.1:3001/health > /dev/null; do
    sleep 1
    RETRY_COUNT=$((RETRY_COUNT + 1))
    if [ $RETRY_COUNT -ge $MAX_RETRIES ]; then
        failure "Service failed to start on $BIND_ADDRESS after $MAX_RETRIES seconds. See verify_service.log for details."
    fi
done

log "Service is ready. Running bash integration tests (itest.sh)..."
./scripts/itest.sh http://127.0.0.1:3001 root_password || failure "itest.sh failed."
success "Integration tests (itest.sh) passed."

log "Running Playwright UI smoke tests..."
npx playwright test tests/ui/smoke.test.js || failure "Playwright UI tests failed."
success "Playwright UI tests passed."

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "==========================================================================="
success "FULL VERIFICATION COMPLETED SUCCESSFULLY!"
echo "==========================================================================="
