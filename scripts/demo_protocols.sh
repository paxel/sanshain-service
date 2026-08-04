#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Protocols Demo Script (AsyncAPI & gRPC/Proto)
#
# Demonstrates:
#   1. Services providing AsyncAPI specifications (Kafka topics/channels)
#   2. Services providing gRPC/Proto definitions (// sanshain-version: marker)
#   3. Clients requiring specific channels and methods at pinned versions
#   4. Cross-protocol dependency tracking (REST -> Kafka, REST -> gRPC)
#   5. Unified dependency graph showing all protocol types
# ============================================================================

BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
ADMIN_USER="${SANSHAIN_USER:-root}"
ADMIN_PASSWORD="${SANSHAIN_PASSWORD:-${INITIAL_ADMIN_PASSWORD:-}}"
TOKEN="${SANSHAIN_TOKEN:-}"

if [ -z "$TOKEN" ] && [ -n "$ADMIN_PASSWORD" ]; then
  echo ">>> Logging in to obtain token..."
  LOGIN_RESPONSE=$(curl -s -X POST "$BASE_URL/auth/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"$ADMIN_USER\", \"password\":\"$ADMIN_PASSWORD\"}")
  TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r .token)
  if [ "$TOKEN" = "null" ]; then
    echo "    Login failed. Proceeding without token (dev_mode must be enabled)."
    TOKEN=""
  else
    echo "    Login successful."
  fi
fi

AUTH_HEADER=""
if [ -n "$TOKEN" ]; then
  AUTH_HEADER="Authorization: Bearer $TOKEN"
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
provide_openapi() {
  local svc="$1" stability="$2" yaml="$3"
  echo ">>> PROVIDE OPENAPI  $svc ($stability)"
  PAYLOAD=$(jq -n --arg s "$svc" --arg st "$stability" --arg y "$yaml" \
    '{producername:$s, stability:$st, openapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

provide_asyncapi() {
  local svc="$1" stability="$2" yaml="$3"
  echo ">>> PROVIDE ASYNCAPI $svc ($stability)"
  PAYLOAD=$(jq -n --arg s "$svc" --arg st "$stability" --arg y "$yaml" \
    '{producername:$s, stability:$st, asyncapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide/asyncapi" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

provide_grpc() {
  local svc="$1" stability="$2" proto="$3"
  echo ">>> PROVIDE GRPC     $svc ($stability)"
  PAYLOAD=$(jq -n --arg s "$svc" --arg st "$stability" --arg p "$proto" \
    '{producername:$s, stability:$st, proto_content:$p}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide/grpc" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

require_endpoint() {
  local client="$1" svc="$2" version="$3" path="$4" method="$5"
  echo ">>> REQUIRE REST     $client -> $svc $method $path @ $version"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require" \
    --data-urlencode "consumername=$client" \
    --data-urlencode "producername=$svc" \
    --data-urlencode "version=$version" \
    --data-urlencode "path=$path" \
    --data-urlencode "method=$method" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  STATUS=$(echo "$RESPONSE" | tail -1)
  echo "    HTTP $STATUS"
}

require_asyncapi() {
  local client="$1" svc="$2" version="$3" channel="$4" op="$5"
  echo ">>> REQUIRE ASYNC    $client -> $svc $op $channel @ $version"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require/asyncapi" \
    --data-urlencode "consumername=$client" \
    --data-urlencode "producername=$svc" \
    --data-urlencode "version=$version" \
    --data-urlencode "path=$channel" \
    --data-urlencode "method=$op" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  STATUS=$(echo "$RESPONSE" | tail -1)
  echo "    HTTP $STATUS"
}

require_grpc() {
  local client="$1" svc="$2" version="$3" service="$4" method="$5"
  echo ">>> REQUIRE GRPC     $client -> $svc $service.$method @ $version"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require/grpc" \
    --data-urlencode "consumername=$client" \
    --data-urlencode "producername=$svc" \
    --data-urlencode "version=$version" \
    --data-urlencode "path=$service" \
    --data-urlencode "method=$method" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  STATUS=$(echo "$RESPONSE" | tail -1)
  echo "    HTTP $STATUS"
}

section() {
  echo ""
  echo "==========================================================================="
  echo "  $1"
  echo "==========================================================================="
  echo ""
}

# ---------------------------------------------------------------------------
# Sample Specifications
# OpenAPI/AsyncAPI carry their version in info.version; proto files carry a
# mandatory "// sanshain-version:" comment that travels into the split files.
# ---------------------------------------------------------------------------

GATEWAY_OPENAPI='openapi: 3.0.0
info:
  title: API Gateway
  version: 1.0.0
paths:
  /orders:
    post:
      summary: Place an order
      responses:
        "201":
          description: Created'

EVENT_BUS_ASYNCAPI='asyncapi: 2.6.0
info:
  title: Event Bus
  version: 1.0.0
channels:
  user-signups:
    publish:
      summary: Receive user signup events
      message:
        name: UserSignedUp
        payload:
          type: object
          properties:
            userId: { type: string }
            timestamp: { type: string }
  order-events:
    publish:
      summary: Publish order-related events
      message:
        name: OrderEvent
        payload:
          type: object
          properties:
            orderId: { type: string }
            status: { type: string }'

USER_PROTO='syntax = "proto3";
// sanshain-version: 1.0.0
package users;

message GetUserRequest { string id = 1; }
message UserResponse { string id = 1; string name = 2; }

service UserService {
  rpc GetUser (GetUserRequest) returns (UserResponse);
  rpc ListUsers (GetUserRequest) returns (UserResponse);
}'

INVENTORY_PROTO='syntax = "proto3";
// sanshain-version: 1.0.0
package inventory;

message StockRequest { string sku = 1; }
message StockResponse { int32 quantity = 1; }

service InventoryService {
  rpc CheckStock (StockRequest) returns (StockResponse);
}'

# ---------------------------------------------------------------------------
# Demo Start
# ---------------------------------------------------------------------------

section "1. Provide Specifications (OpenAPI, AsyncAPI, Proto) as GA 1.0.0"

provide_openapi "api-gateway" "ga" "$GATEWAY_OPENAPI"
provide_asyncapi "event-bus" "ga" "$EVENT_BUS_ASYNCAPI"
provide_grpc "user-service" "ga" "$USER_PROTO"
provide_grpc "inventory-service" "ga" "$INVENTORY_PROTO"

section "2. Register Dependencies (Cross-Protocol, pinned @ 1.0.0)"

# User Service (gRPC) requires notifications from Event Bus (AsyncAPI)
echo "  User Service publishes to user-signups topic..."
require_asyncapi "user-service" "event-bus" "1.0.0" "user-signups" "PUB"

# API Gateway (REST) calls User Service (gRPC)
echo "  API Gateway calls User Service via gRPC..."
require_grpc "api-gateway" "user-service" "1.0.0" "UserService" "GetUser"

# Order Service (New) calls API Gateway (REST) and Inventory Service (gRPC)
# and also listens to order-events (AsyncAPI)
echo "  Order Service (unregistered) consumes from multiple protocols..."
require_endpoint "order-service" "api-gateway" "1.0.0" "/orders" "POST"
require_grpc     "order-service" "inventory-service" "1.0.0" "InventoryService" "CheckStock"
require_asyncapi "order-service" "event-bus" "1.0.0" "order-events" "PUB"

# Analytics Service consumes all events
echo "  Analytics Service consumes all events from Event Bus..."
require_asyncapi "analytics-service" "event-bus" "1.0.0" "user-signups" "PUB"
require_asyncapi "analytics-service" "event-bus" "1.0.0" "order-events" "PUB"

section "3. Verify Reports"

echo ">>> Fetching Markdown Report..."
RESPONSE=$(curl -s -w "\n%{http_code}" "$BASE_URL/report/markdown" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
BODY=$(echo "$RESPONSE" | sed '$d')
STATUS=$(echo "$RESPONSE" | tail -1)
echo "    HTTP $STATUS"
echo "    Preview (Protocol column should be visible):"
echo "$BODY" | head -n 25 | sed 's/^/      /'

echo ""
echo "==========================================================================="
echo "  Protocols Demo Complete!"
echo "  Check the UI to see the integrated dependency graph with:"
echo "    - REST (OpenAPI)"
echo "    - Kafka (AsyncAPI)"
echo "    - gRPC (Proto)"
echo "==========================================================================="
