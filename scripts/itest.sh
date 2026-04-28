#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Integration Test Suite (itest)
#
# This script verifies that the service endpoints behave exactly as expected.
# It performs a full lifecycle of operations: Auth, Provider, Consumer,
# Admin, and Observability.
#
# Usage:
#   ./scripts/itest.sh [base_url] [admin_password]
#
# Default base_url: http://localhost:3000
# Default admin_password: root_password (or INITIAL_ADMIN_PASSWORD env var)
# ============================================================================

BASE_URL="${1:-${SANSHAIN_URL:-http://localhost:3000}}"
ADMIN_USER="root"
ADMIN_PASSWORD="${2:-${INITIAL_ADMIN_PASSWORD:-root_password}}"
TOKEN=""

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; PASSED_TESTS=$((PASSED_TESTS + 1)); }
log_failure() { echo -e "${RED}[FAIL]${NC} $1"; FAILED_TESTS=$((FAILED_TESTS + 1)); }

header() {
    echo ""
    echo "==========================================================================="
    echo "  $1"
    echo "==========================================================================="
}

# captures status and body
# returns status in global LAST_STATUS and body in global LAST_BODY
call_api() {
    local method="$1"
    local path="$2"
    local data="${3:-}"
    local auth="${4:-true}"
    
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    local auth_header=""
    if [ "$auth" = "true" ] && [ -n "$TOKEN" ]; then
        auth_header="-H \"Authorization: Bearer $TOKEN\""
    fi
    
    local cmd="curl -s -w \"\\n%{http_code}\" -X $method \"$BASE_URL$path\""
    if [ -n "$data" ]; then
        cmd="$cmd -H \"Content-Type: application/json\" -d '$data'"
    fi
    
    if [ -n "$auth_header" ]; then
        cmd="$cmd $auth_header"
    fi

    # Execute
    local response
    response=$(eval "$cmd")
    
    LAST_BODY=$(echo "$response" | sed '$d')
    LAST_STATUS=$(echo "$response" | tail -1)
}

assert_status() {
    local expected="$1"
    local msg="$2"
    if [ "$LAST_STATUS" -eq "$expected" ]; then
        log_success "$msg (Status: $LAST_STATUS)"
    else
        log_failure "$msg (Expected $expected, got $LAST_STATUS)"
        echo "      Response: $LAST_BODY"
        # exit 1 # uncomment to fail fast
    fi
}

assert_json() {
    local query="$1"
    local expected="$2"
    local msg="$3"
    
    local actual
    actual=$(echo "$LAST_BODY" | jq -r "$query" 2>/dev/null || echo "ERROR")
    
    if [ "$actual" = "$expected" ]; then
        log_success "$msg ($actual)"
    else
        log_failure "$msg (Expected $expected, got $actual)"
        # exit 1
    fi
}

# ---------------------------------------------------------------------------
# Test Cases
# ---------------------------------------------------------------------------

header "Starting Integration Tests against $BASE_URL"

# 1. Health Check
call_api GET "/health" "" false
assert_status 200 "Service is healthy"

# 2. Authentication
header "Authentication Tests"

# Invalid login
call_api POST "/auth/login" '{"username":"root", "password":"wrong-password"}' false
assert_status 401 "Login with wrong password rejected"

# Valid login
log_info "Attempting login with user: $ADMIN_USER"
call_api POST "/auth/login" "{\"username\":\"$ADMIN_USER\", \"password\":\"$ADMIN_PASSWORD\"}" false
if [ "$LAST_STATUS" -eq 200 ]; then
    TOKEN=$(echo "$LAST_BODY" | jq -r .token)
    log_success "Login successful, token obtained"
else
    log_failure "Login failed. Cannot proceed with authenticated tests."
    echo "      Response: $LAST_BODY"
    exit 1
fi

# 3. Provider API
header "Provider API Tests"

OPENAPI_SPEC="openapi: '3.0.0'
info:
  title: Test Service
  version: '1.0'
paths:
  /hello:
    get:
      responses:
        '200':
          description: OK
  /world:
    post:
      responses:
        '201':
          description: Created"

# PROVIDER_SVC is used to ensure unique service name per run if needed, 
# but here we use it to test idempotency, so we'll just keep it stable 
# or reset the DB if we want a clean run.
# To make it robust for multiple consecutive runs without reset:
TEST_ID=$(date +%s)
SVC_NAME="itest-svc-$TEST_ID"
log_info "Using service name: $SVC_NAME"

# Provide OpenAPI
PAYLOAD=$(jq -n --arg svc "$SVC_NAME" --arg branch "main" --arg yaml "$OPENAPI_SPEC" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')

call_api POST "/provide" "$PAYLOAD"
assert_status 202 "Provide OpenAPI specification"
assert_json ".changes.inserts" "2" "Injected 2 new endpoints"

# Idempotency check
call_api POST "/provide" "$PAYLOAD"
assert_status 202 "Provide same spec again (idempotent)"
assert_json ".changes.inserts" "0" "Zero new endpoints injected (idempotent)"

# Provide AsyncAPI
ASYNC_SPEC="asyncapi: '2.6.0'
info:
  title: Test Async
  version: '1.0'
channels:
  user/signup:
    subscribe:
      message:
        name: UserSignedUp"

PAYLOAD_ASYNC=$(jq -n --arg svc "$SVC_NAME" --arg branch "main" --arg yaml "$ASYNC_SPEC" \
    '{servicename:$svc, branch:$branch, asyncapi_yaml:$yaml}')

call_api POST "/provide/asyncapi" "$PAYLOAD_ASYNC"
assert_status 202 "Provide AsyncAPI specification"

# 4. Consumer API
header "Consumer API Tests"

# Require endpoint (existing)
call_api GET "/require?clientname=itest-client&servicename=$SVC_NAME&branch=main&path=/hello&method=GET"
assert_status 200 "Require existing endpoint"

# Require endpoint (non-existing)
call_api GET "/require?clientname=itest-client&servicename=$SVC_NAME&branch=main&path=/missing&method=GET"
assert_status 404 "Require missing endpoint returns 404"

# Require bundle
BUNDLE_PAYLOAD=$(jq -n --arg client "itest-client" --arg svc "$SVC_NAME" --arg branch "main" \
    '{clientname:$client, servicename:$svc, branch:$branch, endpoints:[{path:"/hello", method:"GET"}, {path:"/world", method:"POST"}]}')

call_api POST "/require-bundle" "$BUNDLE_PAYLOAD"
assert_status 200 "Require bundle (multiple endpoints)"

# 5. Admin API
header "Admin API Tests"

# Developer Mode
call_api GET "/admin/settings/dev-mode"
assert_status 200 "Get developer mode status"
INITIAL_DEV_MODE=$(echo "$LAST_BODY" | jq -r .dev_mode)

# Toggle it
NEW_DEV_MODE=$([ "$INITIAL_DEV_MODE" = "true" ] && echo "false" || echo "true")
call_api POST "/admin/settings/dev-mode" "{\"enabled\":$NEW_DEV_MODE}"
assert_status 200 "Set developer mode to $NEW_DEV_MODE"

call_api GET "/admin/settings/dev-mode"
assert_json ".dev_mode" "$NEW_DEV_MODE" "Developer mode persisted"

# Restore it
call_api POST "/admin/settings/dev-mode" "{\"enabled\":$INITIAL_DEV_MODE}"

# List Users
call_api GET "/admin/users"
assert_status 200 "List users"
assert_json ".[0].username" "$ADMIN_USER" "Root user found in user list"

# Nuke branch (Dry run with invalid confirmation first)
call_api POST "/admin/nuke/branch/main" '{"confirmation":"WRONG"}'
assert_status 400 "Nuke rejected with wrong confirmation"

# Real nuke (be careful, but this is a test service)
# Actually, let's create a temporary branch and nuke it
BRANCH_TO_NUKE="temp-branch-$TEST_ID"
PAYLOAD_TEMP=$(jq -n --arg svc "$SVC_NAME" --arg branch "$BRANCH_TO_NUKE" --arg yaml "$OPENAPI_SPEC" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_TEMP"

call_api POST "/admin/nuke/branch/$BRANCH_TO_NUKE" "{\"confirmation\":\"DELETE BRANCH $BRANCH_TO_NUKE\"}"
assert_status 200 "Nuke temporary branch"
assert_json ".deleted" "1" "Deleted 1 service branch during nuke"

# Real Database Nuke (Factory Reset)
header "Database Nuke (Factory Reset) Tests"

call_api POST "/admin/nuke/database" '{"confirmation":"NUKE DATABASE"}'
assert_status 200 "Nuke database (factory reset)"

# Verify protected branches still exist
call_api GET "/admin/protected-branches"
assert_status 200 "Get protected branches after nuke"
assert_json ". | length > 0" "true" "Protected branches preserved"

# Verify services are gone
call_api GET "/admin/services"
assert_status 200 "Get services after nuke"
assert_json ". | length" "0" "Services cleared"

# 6. Observability
header "Observability Tests"

call_api GET "/metrics" "" false
assert_status 200 "Metrics endpoint accessible"

call_api GET "/admin/observability/stats"
assert_status 200 "Observability stats accessible"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "==========================================================================="
echo "  TEST SUMMARY"
echo "==========================================================================="
echo -e "  Total Tests:   $TOTAL_TESTS"
echo -e "  Passed:        ${GREEN}$PASSED_TESTS${NC}"
echo -e "  Failed:        $([ $FAILED_TESTS -gt 0 ] && echo -e "${RED}$FAILED_TESTS${NC}" || echo -e "${GREEN}0${NC}")"
echo "==========================================================================="

if [ "$FAILED_TESTS" -eq 0 ]; then
    echo -e "${GREEN}SUCCESS: All integration tests passed!${NC}"
    exit 0
else
    echo -e "${RED}FAILURE: $FAILED_TESTS test(s) failed.${NC}"
    exit 1
fi
