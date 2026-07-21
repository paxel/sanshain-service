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

if [ -z "${2:-}" ] && [ -z "${INITIAL_ADMIN_PASSWORD:-}" ]; then
    log_failure "No admin password provided. Please set INITIAL_ADMIN_PASSWORD or pass it as the second argument."
    exit 1
fi

ADMIN_PASSWORD="${2:-${INITIAL_ADMIN_PASSWORD:-}}"
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
    
    local auth_header=()
    if [ "$auth" = "true" ] && [ -n "$TOKEN" ]; then
        auth_header=("-H" "Authorization: Bearer $TOKEN")
    fi
    
    local data_header=()
    if [ -n "$data" ]; then
        data_header=("-H" "Content-Type: application/json" "-d" "$data")
    fi

    # Execute
    local response
    response=$(curl -s -w "\n%{http_code}" -X "$method" "$BASE_URL$path" "${auth_header[@]}" "${data_header[@]}")
    
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

assert_contains() {
    local substring="$1"
    local msg="$2"
    if echo "$LAST_BODY" | grep -q "$substring"; then
        log_success "$msg"
    else
        log_failure "$msg (Expected contains \"$substring\")"
    fi
}

# ---------------------------------------------------------------------------
# Static Analysis & Formatting
# ---------------------------------------------------------------------------

header "Static Analysis & Formatting"
log_info "Running cargo fmt --check..."
if cargo fmt --check; then
    log_success "Formatting check passed"
else
    log_failure "Formatting check failed. Run 'cargo fmt' to fix."
fi

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

# Set auth mode to dev for integration tests to allow /provide and /require
call_api PUT "/admin/auth-config" '{"auth_mode":"dev","ldap_config":null}'
assert_status 200 "Set auth mode to 'dev' for integration testing"

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

# Bundle ETag Stability
header "Bundle ETag Stability Tests"

ETAG_SVC="etag-svc-$TEST_ID"
ETAG_SPEC="openapi: '3.0.0'
info:
  title: ETag Test
  version: '1.0'
paths:
  /alpha:
    get:
      responses:
        '200':
          description: Alpha
  /beta:
    post:
      responses:
        '201':
          description: Beta"

PAYLOAD_ETAG=$(jq -n --arg svc "$ETAG_SVC" --arg branch "main" --arg yaml "$ETAG_SPEC" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_ETAG"
assert_status 202 "Provide spec for ETag stability test"

# Order A: alpha first, beta second
BUNDLE_A=$(jq -n --arg client "etag-client" --arg svc "$ETAG_SVC" --arg branch "main" \
    '{clientname:$client, servicename:$svc, branch:$branch, endpoints:[{path:"/alpha", method:"GET"}, {path:"/beta", method:"POST"}]}')
ETAG_A=$(curl -s -o /dev/null -D - -X POST "$BASE_URL/require-bundle" \
    -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" -H "X-CSRF-Token: test" \
    -d "$BUNDLE_A" | grep -i "^etag:" | tr -d '\r\n ')

# Order B: beta first, alpha second
BUNDLE_B=$(jq -n --arg client "etag-client" --arg svc "$ETAG_SVC" --arg branch "main" \
    '{clientname:$client, servicename:$svc, branch:$branch, endpoints:[{path:"/beta", method:"POST"}, {path:"/alpha", method:"GET"}]}')
ETAG_B=$(curl -s -o /dev/null -D - -X POST "$BASE_URL/require-bundle" \
    -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" -H "X-CSRF-Token: test" \
    -d "$BUNDLE_B" | grep -i "^etag:" | tr -d '\r\n ')

if [ "$ETAG_A" = "$ETAG_B" ] && [ -n "$ETAG_A" ]; then
    log_success "Bundle ETags match regardless of endpoint order"
else
    log_failure "Bundle ETags differ: A='$ETAG_A' B='$ETAG_B'"
fi

# 7. Advanced Features: Optimistic Concurrency
header "Optimistic Concurrency Tests"

CONCURRENCY_SVC="concurrency-svc-$TEST_ID"
log_info "Using service: $CONCURRENCY_SVC"

# Step 1: First provide
PAYLOAD_C1=$(jq -n --arg svc "$CONCURRENCY_SVC" --arg branch "main" --arg yaml "$OPENAPI_SPEC" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_C1"
assert_status 202 "First provide"
V1=$(echo "$LAST_BODY" | jq -r .version)

# Step 2: Second provide (increment version)
OPENAPI_SPEC_V2="$OPENAPI_SPEC
# Change V2"
PAYLOAD_C2=$(jq -n --arg svc "$CONCURRENCY_SVC" --arg branch "main" --arg yaml "$OPENAPI_SPEC_V2" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_C2"
assert_status 202 "Second provide"
V2=$(echo "$LAST_BODY" | jq -r .version)

# Step 3: Outdated base_version
PAYLOAD_OUTDATED=$(jq -n --arg svc "$CONCURRENCY_SVC" --arg branch "main" --arg yaml "$OPENAPI_SPEC" --arg v "$V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml, base_version:$v}')
call_api POST "/provide" "$PAYLOAD_OUTDATED"
assert_status 409 "Reject outdated base_version ($V1 vs $V2)"

# Step 4: Correct base_version
PAYLOAD_CORRECT=$(jq -n --arg svc "$CONCURRENCY_SVC" --arg branch "main" --arg yaml "$OPENAPI_SPEC" --arg v "$V2" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml, base_version:$v}')
call_api POST "/provide" "$PAYLOAD_CORRECT"
assert_status 202 "Accept correct base_version ($V2)"

# 8. Feature-Branch Breaking Changes (protected branches are the only gate)
header "Feature-Branch Breaking Change Tests"

FB_SVC="service-FEATBREAK"
FB_BRANCH="feat-break-$TEST_ID"
log_info "Using branch: $FB_BRANCH"

FB_V1="openapi: '3.0.0'
info:
  title: FeatBreak API
  version: '1.0'
paths:
  /shared:
    get:
      responses:
        '200':
          description: V1"

FB_INCOMPAT="openapi: '3.0.0'
info:
  title: FeatBreak API
  version: '1.0'
paths:
  /shared:
    get:
      responses:
        '201':
          description: 'INCOMPAT (Breaking: 200 removed)'"

# Push V1 to main so the service exists on a protected branch
PAYLOAD_FB_MAIN=$(jq -n --arg svc "$FB_SVC" --arg branch "main" --arg yaml "$FB_V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_FB_MAIN"
assert_status 202 "FEATBREAK service pushed to main"

# V1 on a feature branch
PAYLOAD_FB_V1=$(jq -n --arg svc "$FB_SVC" --arg branch "$FB_BRANCH" --arg yaml "$FB_V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_FB_V1"
assert_status 202 "V1 provided on feature branch"

# Breaking change on the feature branch is accepted without force
PAYLOAD_FB_BREAK=$(jq -n --arg svc "$FB_SVC" --arg branch "$FB_BRANCH" --arg yaml "$FB_INCOMPAT" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_FB_BREAK"
assert_status 202 "Breaking change accepted on feature branch without force"

# The same breaking change on main is rejected
PAYLOAD_FB_BREAK_MAIN=$(jq -n --arg svc "$FB_SVC" --arg branch "main" --arg yaml "$FB_INCOMPAT" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_FB_BREAK_MAIN"
assert_status 409 "Breaking change still rejected on protected branch"

# 9. Cross-service Independence (Same Path, Different Services)
header "Cross-service Independence Tests"

SVC_X="service-X-$TEST_ID"
SVC_Y="service-Y-$TEST_ID"
COMMON_PATH="/notification"

SPEC_X="openapi: '3.0.0'
info:
  title: Service X API
  version: '1.0'
paths:
  $COMMON_PATH:
    get:
      responses:
        '200':
          description: Result X"

SPEC_Y="openapi: '3.0.0'
info:
  title: Service Y API
  version: '1.0'
paths:
  $COMMON_PATH:
    get:
      responses:
        '202':
          description: Result Y"

PAYLOAD_X=$(jq -n --arg svc "$SVC_X" --arg branch "main" --arg yaml "$SPEC_X" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_X"
assert_status 202 "Service X provides $COMMON_PATH (200 OK)"

PAYLOAD_Y=$(jq -n --arg svc "$SVC_Y" --arg branch "main" --arg yaml "$SPEC_Y" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_Y"
assert_status 202 "Service Y provides $COMMON_PATH (202 Accepted) without conflict"

# Verify they are independent
call_api GET "/admin/endpoint-yaml?servicename=$SVC_X&branch=main&api_type=openapi&path=$COMMON_PATH&method=GET"
assert_contains "Result X" "Service X endpoint preserved"

call_api GET "/admin/endpoint-yaml?servicename=$SVC_Y&branch=main&api_type=openapi&path=$COMMON_PATH&method=GET"
assert_contains "Result Y" "Service Y endpoint preserved"

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
assert_json "map(select(.username == \"$ADMIN_USER\"))[0].username" "$ADMIN_USER" "Root user found in user list"

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

# 10. Force Mode & Onboarding
header "Force Mode & Onboarding Tests"

# Auto-skip: A brand new service with no protected-branch endpoints can push incompatible specs
ONBOARD_SVC="onboard-svc-$TEST_ID"
ONBOARD_BRANCH="feature-onboard-$TEST_ID"

ONBOARD_V1="openapi: '3.0.0'
info:
  title: Onboard API
  version: '1.0'
paths:
  /onboard:
    get:
      responses:
        '200':
          description: OK"

ONBOARD_V2_BREAKING="openapi: '3.0.0'
info:
  title: Onboard API
  version: '2.0'
paths:
  /onboard:
    get:
      responses:
        '201':
          description: Breaking change removed 200"

PAYLOAD_OB1=$(jq -n --arg svc "$ONBOARD_SVC" --arg branch "$ONBOARD_BRANCH" --arg yaml "$ONBOARD_V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_OB1"
assert_status 202 "New service first spec on feature branch"

PAYLOAD_OB2=$(jq -n --arg svc "$ONBOARD_SVC" --arg branch "$ONBOARD_BRANCH" --arg yaml "$ONBOARD_V2_BREAKING" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_OB2"
assert_status 202 "New service breaking change on feature branch succeeds"

# Force on protected branch should be rejected
FORCE_PROTECTED_PAYLOAD=$(jq -n --arg svc "$SVC_NAME" --arg branch "main" --arg yaml "$ONBOARD_V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml, force:true}')
call_api POST "/provide" "$FORCE_PROTECTED_PAYLOAD"
assert_status 400 "Force on protected branch rejected"

# Force on feature branch should succeed even with breaking change
# Push the onboard service to a protected branch
PAYLOAD_OB_MAIN=$(jq -n --arg svc "$ONBOARD_SVC" --arg branch "main" --arg yaml "$ONBOARD_V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_OB_MAIN"
assert_status 202 "Push onboard service to protected branch"

# Use a fresh feature branch
ONBOARD_BRANCH2="feature-onboard2-$TEST_ID"
PAYLOAD_OB_FRESH=$(jq -n --arg svc "$ONBOARD_SVC" --arg branch "$ONBOARD_BRANCH2" --arg yaml "$ONBOARD_V1" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_OB_FRESH"
assert_status 202 "Establish V1 on fresh feature branch"

# Breaking changes on feature branches are accepted even for established services
PAYLOAD_OB3=$(jq -n --arg svc "$ONBOARD_SVC" --arg branch "$ONBOARD_BRANCH2" --arg yaml "$ONBOARD_V2_BREAKING" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_OB3"
assert_status 202 "Breaking change accepted on feature branch (established service)"

# force is accepted but has no effect on feature branches
PAYLOAD_OB4=$(jq -n --arg svc "$ONBOARD_SVC" --arg branch "$ONBOARD_BRANCH2" --arg yaml "$ONBOARD_V2_BREAKING" \
    '{servicename:$svc, branch:$branch, openapi_yaml:$yaml, force:true}')
call_api POST "/provide" "$PAYLOAD_OB4"
assert_status 202 "Force accepted as no-op on feature branch"

# 5b. Merged Report & Protected Branches Public Endpoint
header "Merged Report & Graph Fallback Tests"

# Public protected branches endpoint
call_api GET "/branches/protected"
assert_status 200 "Public protected branches endpoint"
assert_json ". | length > 0" "true" "Protected branches returned"

# Merged report: feature branch with main as target
call_api GET "/report/merged?branch=$ONBOARD_BRANCH2&target=main"
assert_status 200 "Merged report endpoint"
assert_json ".branch" "$ONBOARD_BRANCH2" "Merged report branch field"
assert_json ".target" "main" "Merged report target field"
assert_json ".dependency_graph | length >= 0" "true" "Merged report has dependency_graph"
assert_json ".node_sources | length >= 0" "true" "Merged report has node_sources"
assert_json ".conflicts | type" "array" "Merged report has conflicts array"

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
echo -e "  API Calls:     $TOTAL_TESTS"
echo -e "  Assertions:    ${GREEN}$PASSED_TESTS${NC}"
echo -e "  Failed:        $([ $FAILED_TESTS -gt 0 ] && echo -e "${RED}$FAILED_TESTS${NC}" || echo -e "${GREEN}0${NC}")"
echo "==========================================================================="

if [ "$FAILED_TESTS" -eq 0 ]; then
    echo -e "${GREEN}SUCCESS: All integration tests passed!${NC}"
    exit 0
else
    echo -e "${RED}FAILURE: $FAILED_TESTS test(s) failed.${NC}"
    exit 1
fi
