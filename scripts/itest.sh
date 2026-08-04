#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Integration Test Suite (itest)
#
# This script verifies that the service endpoints behave exactly as expected
# under the 2.0 version-line contract (ADR-0003): producer-declared versions,
# declared stability, GA immutability and exact-pin resolution.
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
    echo "No admin password provided. Please set INITIAL_ADMIN_PASSWORD or pass it as the second argument."
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

# fetches a single response header (lowercased name) into LAST_HEADER
call_api_header() {
    local method="$1"
    local path="$2"
    local header_name="$3"

    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    LAST_HEADER=$(curl -s -o /dev/null -D - -X "$method" "$BASE_URL$path" \
        -H "Authorization: Bearer $TOKEN" \
        | tr -d '\r' | grep -i "^$header_name:" | head -1 | cut -d' ' -f2-)
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

# Valid login. The session token authenticates every /provide and /require
# below — 2.0 needs no dev-mode switch for that.
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

# The version lives in the spec document itself (info.version), and every
# Provide declares its stability: snapshot (overwritable) or ga (immutable).
OPENAPI_SPEC="openapi: '3.0.0'
info:
  title: Test Service
  version: 1.0.0
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

TEST_ID=$(date +%s)
SVC_NAME="itest-svc-$TEST_ID"
CLIENT_NAME="itest-client-$TEST_ID"
log_info "Using service name: $SVC_NAME"

# Provide OpenAPI as a snapshot
PAYLOAD=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$OPENAPI_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"snapshot"}')

call_api POST "/provide" "$PAYLOAD"
assert_status 202 "Provide OpenAPI specification as snapshot"
assert_json ".version" "1.0.0" "Version read from info.version"
assert_json ".stability" "snapshot" "Declared stability echoed"
assert_json ".changes.inserts" "2" "Injected 2 new endpoints"

# Idempotency check: identical content is a no-op, regardless of stability
call_api POST "/provide" "$PAYLOAD"
assert_status 202 "Provide same spec again (idempotent)"
assert_json ".changes.inserts" "0" "Zero new endpoints injected (idempotent)"
assert_json ".version" "1.0.0" "Version unchanged by an idempotent re-provide"

# Missing stability is rejected naming the field
PAYLOAD_NO_STABILITY=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$OPENAPI_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml}')
call_api POST "/provide" "$PAYLOAD_NO_STABILITY"
assert_status 422 "Provide without stability rejected"
assert_contains "missing field \`stability\`" "Rejection names the missing stability field"

# 1.x branch-era fields are rejected by name
PAYLOAD_BRANCH=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$OPENAPI_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"snapshot", branch:"main"}')
call_api POST "/provide" "$PAYLOAD_BRANCH"
assert_status 422 "Provide with 1.x branch field rejected"
assert_contains "unknown field \`branch\`" "Rejection names the unknown branch field"

# A loose version is rejected with guidance
LOOSE_SPEC="${OPENAPI_SPEC/version: 1.0.0/version: '1.0'}"
PAYLOAD_LOOSE=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$LOOSE_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"snapshot"}')
call_api POST "/provide" "$PAYLOAD_LOOSE"
assert_status 400 "Provide with loose version '1.0' rejected"
assert_contains "MAJOR.MINOR.PATCH" "Rejection explains the expected version format"

# A -SNAPSHOT suffix is rejected pointing at the stability flag
SUFFIX_SPEC="${OPENAPI_SPEC/version: 1.0.0/version: 1.0.0-SNAPSHOT}"
PAYLOAD_SUFFIX=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$SUFFIX_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"snapshot"}')
call_api POST "/provide" "$PAYLOAD_SUFFIX"
assert_status 400 "Provide with -SNAPSHOT version suffix rejected"
assert_contains "stability" "Rejection points at the stability flag"

# Provide AsyncAPI
ASYNC_SPEC="asyncapi: '2.6.0'
info:
  title: Test Async
  version: 1.0.0
channels:
  user/signup:
    publish:
      message:
        name: UserSignedUp
        payload:
          type: object"

PAYLOAD_ASYNC=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$ASYNC_SPEC" \
    '{producername:$svc, asyncapi_yaml:$yaml, stability:"snapshot"}')

call_api POST "/provide/asyncapi" "$PAYLOAD_ASYNC"
assert_status 202 "Provide AsyncAPI specification"

# Provide gRPC: the version is a mandatory comment marker
PROTO_NO_MARKER='syntax = "proto3";
message Req {}
message Res {}
service Greeter { rpc Hello (Req) returns (Res); }'
PAYLOAD_PROTO_NO_MARKER=$(jq -n --arg svc "$SVC_NAME" --arg proto "$PROTO_NO_MARKER" \
    '{producername:$svc, proto_content:$proto, stability:"snapshot"}')
call_api POST "/provide/grpc" "$PAYLOAD_PROTO_NO_MARKER"
assert_status 400 "Provide proto without sanshain-version marker rejected"
assert_contains "sanshain-version" "Rejection shows the marker syntax"

PROTO_WITH_MARKER="// sanshain-version: 1.0.0
$PROTO_NO_MARKER"
PAYLOAD_PROTO=$(jq -n --arg svc "$SVC_NAME" --arg proto "$PROTO_WITH_MARKER" \
    '{producername:$svc, proto_content:$proto, stability:"snapshot"}')
call_api POST "/provide/grpc" "$PAYLOAD_PROTO"
assert_status 202 "Provide proto with version marker"
assert_json ".version" "1.0.0" "Proto version read from the marker"

# 4. Version Lifecycle: snapshot → promote → immutable
header "Version Lifecycle Tests"

# Promote the snapshot to GA by re-providing the same content as ga
PAYLOAD_GA=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$OPENAPI_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"ga"}')
call_api POST "/provide" "$PAYLOAD_GA"
assert_status 202 "Promote snapshot 1.0.0 to GA"
assert_json ".stability" "ga" "Promotion flips the stability"
assert_json ".version" "1.0.0" "Promotion keeps the version number"

# The version listing shows the promoted line
call_api GET "/producers/$SVC_NAME/versions?api_type=openapi"
assert_status 200 "List producer versions"
assert_json '.[0].version' "1.0.0" "Version line lists 1.0.0"
assert_json '.[0].stability' "ga" "Version line shows stability ga"
assert_json '.[0].endpoint_count' "2" "Version line counts its endpoints"

# The forgotten-bump mistake is caught at the door with a proposed remedy
OPENAPI_SPEC_CHANGED="${OPENAPI_SPEC/description: OK/description: Greeting}"
PAYLOAD_FORGOT=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$OPENAPI_SPEC_CHANGED" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"ga"}')
call_api POST "/provide" "$PAYLOAD_FORGOT"
assert_status 409 "GA 1.0.0 with different content rejected (forgotten bump)"
assert_contains "immutable" "Rejection explains GA immutability"
PROPOSED=$(echo "$LAST_BODY" | jq -r .proposed_version)
if [ -n "$PROPOSED" ] && [ "$PROPOSED" != "null" ]; then
    log_success "Rejection proposes a free version ($PROPOSED)"
else
    log_failure "Rejection carries no proposed_version"
fi

# A GA'd number can never carry a snapshot again
PAYLOAD_SNAP_ON_GA=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$OPENAPI_SPEC_CHANGED" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"snapshot"}')
call_api POST "/provide" "$PAYLOAD_SNAP_ON_GA"
assert_status 409 "Snapshot for the GA'd number rejected"
assert_contains "can never carry a snapshot again" "Rejection explains the permanent claim"

# Publishing under the proposed version succeeds
SPEC_BUMPED="${OPENAPI_SPEC_CHANGED/version: 1.0.0/version: $PROPOSED}"
PAYLOAD_BUMPED=$(jq -n --arg svc "$SVC_NAME" --arg yaml "$SPEC_BUMPED" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"ga"}')
call_api POST "/provide" "$PAYLOAD_BUMPED"
assert_status 202 "Publish the changed content as the proposed version"
assert_json ".version" "$PROPOSED" "Bumped version stored"

# 5. Consumer API
header "Consumer API Tests"

# Require an endpoint at an exact pin
call_api GET "/require?consumername=$CLIENT_NAME&producername=$SVC_NAME&version=1.0.0&path=/hello&method=GET"
assert_status 200 "Require existing endpoint at pinned version 1.0.0"

# The resolution is named in the response headers
call_api_header GET "/require?consumername=$CLIENT_NAME&producername=$SVC_NAME&version=1.0.0&path=/hello&method=GET" "x-sanshain-stability"
if [ "$LAST_HEADER" = "ga" ]; then
    log_success "X-Sanshain-Stability names the serving stability (ga)"
else
    log_failure "X-Sanshain-Stability expected 'ga', got '$LAST_HEADER'"
fi

# An endpoint the pinned version deliberately does not include: 410 Gone
call_api GET "/require?consumername=$CLIENT_NAME&producername=$SVC_NAME&version=1.0.0&path=/missing&method=GET"
assert_status 410 "Require endpoint absent from the pinned version returns 410"

# An unknown pinned version is a configuration error: immediate 404
call_api GET "/require?consumername=$CLIENT_NAME&producername=$SVC_NAME&version=9.9.9&path=/hello&method=GET"
assert_status 404 "Require unknown pinned version returns 404"
assert_contains "configuration error" "404 explains the missing pin"

# 1.x branch parameter is rejected by name
call_api GET "/require?consumername=$CLIENT_NAME&producername=$SVC_NAME&version=1.0.0&path=/hello&method=GET&branch=main"
assert_status 400 "Require with 1.x branch parameter rejected"
assert_contains "unknown field \`branch\`" "Rejection names the unknown parameter"

# Require bundle
BUNDLE_PAYLOAD=$(jq -n --arg client "$CLIENT_NAME" --arg svc "$SVC_NAME" \
    '{consumername:$client, producername:$svc, version:"1.0.0", endpoints:[{path:"/hello", method:"GET"}, {path:"/world", method:"POST"}]}')
call_api POST "/require-bundle" "$BUNDLE_PAYLOAD"
assert_status 200 "Require bundle (multiple endpoints)"

# Bundle with a missing endpoint lists it and answers 410
BUNDLE_MISSING=$(jq -n --arg client "$CLIENT_NAME" --arg svc "$SVC_NAME" \
    '{consumername:$client, producername:$svc, version:"1.0.0", endpoints:[{path:"/hello", method:"GET"}, {path:"/missing", method:"GET"}]}')
call_api POST "/require-bundle" "$BUNDLE_MISSING"
assert_status 410 "Require bundle with missing endpoint returns 410"
assert_contains "GET /missing" "410 lists the missing endpoints"

# The report records the successful pins with version and stability
call_api GET "/report"
assert_status 200 "Dependency report accessible"
assert_json "[.dependency_graph[] | select(.client==\"$CLIENT_NAME\" and .path==\"/hello\")][0].version" "1.0.0" "Report edge carries the pinned version"
assert_json "[.dependency_graph[] | select(.client==\"$CLIENT_NAME\" and .path==\"/hello\")][0].stability" "ga" "Report edge carries the stability"

# Bundle ETag Stability
header "Bundle ETag Stability Tests"

ETAG_SVC="etag-svc-$TEST_ID"
ETAG_SPEC="openapi: '3.0.0'
info:
  title: ETag Test
  version: 1.0.0
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

PAYLOAD_ETAG=$(jq -n --arg svc "$ETAG_SVC" --arg yaml "$ETAG_SPEC" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"ga"}')
call_api POST "/provide" "$PAYLOAD_ETAG"
assert_status 202 "Provide spec for ETag stability test"

# Order A: alpha first, beta second
BUNDLE_A=$(jq -n --arg client "etag-client" --arg svc "$ETAG_SVC" \
    '{consumername:$client, producername:$svc, version:"1.0.0", endpoints:[{path:"/alpha", method:"GET"}, {path:"/beta", method:"POST"}]}')
ETAG_A=$(curl -s -o /dev/null -D - -X POST "$BASE_URL/require-bundle" \
    -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
    -d "$BUNDLE_A" | grep -i "^etag:" | tr -d '\r\n ')

# Order B: beta first, alpha second
BUNDLE_B=$(jq -n --arg client "etag-client" --arg svc "$ETAG_SVC" \
    '{consumername:$client, producername:$svc, version:"1.0.0", endpoints:[{path:"/beta", method:"POST"}, {path:"/alpha", method:"GET"}]}')
ETAG_B=$(curl -s -o /dev/null -D - -X POST "$BASE_URL/require-bundle" \
    -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
    -d "$BUNDLE_B" | grep -i "^etag:" | tr -d '\r\n ')

if [ "$ETAG_A" = "$ETAG_B" ] && [ -n "$ETAG_A" ]; then
    log_success "Bundle ETags match regardless of endpoint order"
else
    log_failure "Bundle ETags differ: A='$ETAG_A' B='$ETAG_B'"
fi

# 6. Cross-service Independence (Same Path, Different Services)
header "Cross-service Independence Tests"

SVC_X="service-X-$TEST_ID"
SVC_Y="service-Y-$TEST_ID"
COMMON_PATH="/notification"

SPEC_X="openapi: '3.0.0'
info:
  title: Service X API
  version: 1.0.0
paths:
  $COMMON_PATH:
    get:
      responses:
        '200':
          description: Result X"

SPEC_Y="openapi: '3.0.0'
info:
  title: Service Y API
  version: 1.0.0
paths:
  $COMMON_PATH:
    get:
      responses:
        '202':
          description: Result Y"

PAYLOAD_X=$(jq -n --arg svc "$SVC_X" --arg yaml "$SPEC_X" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"ga"}')
call_api POST "/provide" "$PAYLOAD_X"
assert_status 202 "Service X provides $COMMON_PATH (200 OK)"

PAYLOAD_Y=$(jq -n --arg svc "$SVC_Y" --arg yaml "$SPEC_Y" \
    '{producername:$svc, openapi_yaml:$yaml, stability:"ga"}')
call_api POST "/provide" "$PAYLOAD_Y"
assert_status 202 "Service Y provides $COMMON_PATH (202 Accepted) without conflict"

# Verify they are independent
call_api GET "/admin/endpoint-yaml?producername=$SVC_X&version=1.0.0&api_type=openapi&path=$COMMON_PATH&method=GET"
assert_contains "Result X" "Service X endpoint preserved"

call_api GET "/admin/endpoint-yaml?producername=$SVC_Y&version=1.0.0&api_type=openapi&path=$COMMON_PATH&method=GET"
assert_contains "Result Y" "Service Y endpoint preserved"

# 7. Admin API
header "Admin API Tests"

# List Users
call_api GET "/admin/users"
assert_status 200 "List users"
assert_json "map(select(.username == \"$ADMIN_USER\"))[0].username" "$ADMIN_USER" "Root user found in user list"

# Snapshot expiry settings (use-based; replaces 1.x branch max age)
call_api GET "/admin/settings/snapshot-max-age"
assert_status 200 "Get snapshot max age"
INITIAL_SNAPSHOT_AGE=$(echo "$LAST_BODY" | jq -r .days)

call_api POST "/admin/settings/snapshot-max-age" '{"days":45}'
assert_status 200 "Set snapshot max age to 45"
call_api GET "/admin/settings/snapshot-max-age"
assert_json ".days" "45" "Snapshot max age persisted"
call_api POST "/admin/settings/snapshot-max-age" "{\"days\":$INITIAL_SNAPSHOT_AGE}"
assert_status 200 "Restore snapshot max age"

# Manual snapshot cleanup trigger reports what it deleted
call_api POST "/admin/cleanup/snapshots" '{}'
assert_status 200 "Trigger snapshot cleanup"
DELETED=$(echo "$LAST_BODY" | jq -r .deleted)
if [ "$DELETED" != "null" ] && [ "$DELETED" -ge 0 ] 2>/dev/null; then
    log_success "Cleanup reports deleted snapshot count ($DELETED)"
else
    log_failure "Cleanup response lacks a numeric .deleted (got '$DELETED')"
fi

# Dependency max age survives unchanged from 1.x
call_api GET "/admin/settings/dependency-max-age"
assert_status 200 "Get dependency max age"
assert_json ".days | type" "number" "Dependency max age is numeric"

# Real Database Nuke (Factory Reset)
header "Database Nuke (Factory Reset) Tests"

call_api POST "/admin/nuke/database" '{"confirmation":"WRONG"}'
assert_status 400 "Nuke rejected with wrong confirmation"

call_api POST "/admin/nuke/database" '{"confirmation":"NUKE DATABASE"}'
assert_status 200 "Nuke database (factory reset)"

# Verify services are gone
call_api GET "/admin/producers"
assert_status 200 "Get services after nuke"
assert_json ". | length" "0" "Services cleared"

# 8. Observability
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
