#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Protocols Demo Script (AsyncAPI & gRPC/Proto)
#
# Demonstrates:
#   1. Services providing AsyncAPI specifications (Kafka topics/channels)
#   2. Services providing gRPC/Proto definitions
#   3. Clients requiring specific channels and methods
#   4. Cross-protocol dependency tracking (REST -> Kafka, REST -> gRPC)
#   5. Unified dependency graph showing all protocol types
# ============================================================================

BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
ADMIN_USER="${SANSHAIN_USER:-root}"
ADMIN_PASSWORD="${SANSHAIN_PASSWORD:-}"
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
  local svc="$1" branch="$2" yaml="$3"
  echo ">>> PROVIDE OPENAPI  $svc @ $branch"
  PAYLOAD=$(jq -n --arg s "$svc" --arg b "$branch" --arg y "$yaml" \
    '{servicename:$s, branch:$b, openapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

provide_asyncapi() {
  local svc="$1" branch="$2" yaml="$3"
  echo ">>> PROVIDE ASYNCAPI $svc @ $branch"
  PAYLOAD=$(jq -n --arg s "$svc" --arg b "$branch" --arg y "$yaml" \
    '{servicename:$s, branch:$b, asyncapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide/asyncapi" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

provide_grpc() {
  local svc="$1" branch="$2" proto="$3"
  echo ">>> PROVIDE GRPC     $svc @ $branch"
  PAYLOAD=$(jq -n --arg s "$svc" --arg b "$branch" --arg p "$proto" \
    '{servicename:$s, branch:$b, proto_content:$p}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide/grpc" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

require_endpoint() {
  local client="$1" svc="$2" branch="$3" path="$4" method="$5"
  echo ">>> REQUIRE REST     $client -> $svc $method $path ($branch)"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require" \
    --data-urlencode "clientname=$client" \
    --data-urlencode "servicename=$svc" \
    --data-urlencode "branch=$branch" \
    --data-urlencode "path=$path" \
    --data-urlencode "method=$method" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  STATUS=$(echo "$RESPONSE" | tail -1)
  echo "    HTTP $STATUS"
}

require_asyncapi() {
  local client="$1" svc="$2" branch="$3" channel="$4" op="$5"
  echo ">>> REQUIRE ASYNC    $client -> $svc $op $channel ($branch)"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require/asyncapi" \
    --data-urlencode "clientname=$client" \
    --data-urlencode "servicename=$svc" \
    --data-urlencode "branch=$branch" \
    --data-urlencode "path=$channel" \
    --data-urlencode "method=$op" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  STATUS=$(echo "$RESPONSE" | tail -1)
  echo "    HTTP $STATUS"
}

require_grpc() {
  local client="$1" svc="$2" branch="$3" service="$4" method="$5"
  echo ">>> REQUIRE GRPC     $client -> $svc $service.$method ($branch)"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require/grpc" \
    --data-urlencode "clientname=$client" \
    --data-urlencode "servicename=$svc" \
    --data-urlencode "branch=$branch" \
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
        payload:
          type: object
          properties:
            userId: { type: string }
            timestamp: { type: string }
  order-events:
    publish:
      summary: Publish order-related events
      message:
        payload:
          type: object
          properties:
            orderId: { type: string }
            status: { type: string }'

USER_PROTO='syntax = "proto3";
package users;

message GetUserRequest { string id = 1; }
message UserResponse { string id = 1; string name = 2; }

service UserService {
  rpc GetUser (GetUserRequest) returns (UserResponse);
  rpc ListUsers (GetUserRequest) returns (UserResponse);
}'

INVENTORY_PROTO='syntax = "proto3";
package inventory;

message StockRequest { string sku = 1; }
message StockResponse { int32 quantity = 1; }

service InventoryService {
  rpc CheckStock (StockRequest) returns (StockResponse);
}'

# ---------------------------------------------------------------------------
# Demo Start
# ---------------------------------------------------------------------------

section "1. Provide Specifications (OpenAPI, AsyncAPI, Proto)"

provide_openapi "api-gateway" "main" "$GATEWAY_OPENAPI"
provide_asyncapi "event-bus" "main" "$EVENT_BUS_ASYNCAPI"
provide_grpc "user-service" "main" "$USER_PROTO"
provide_grpc "inventory-service" "main" "$INVENTORY_PROTO"

section "2. Register Dependencies (Cross-Protocol)"

# User Service (gRPC) requires notifications from Event Bus (AsyncAPI)
echo "  User Service publishes to user-signups topic..."
require_asyncapi "user-service" "event-bus" "main" "user-signups" "PUB"

# API Gateway (REST) calls User Service (gRPC)
echo "  API Gateway calls User Service via gRPC..."
require_grpc "api-gateway" "user-service" "main" "UserService" "GetUser"

# Order Service (New) calls API Gateway (REST) and Inventory Service (gRPC)
# and also listens to order-events (AsyncAPI)
echo "  Order Service (unregistered) consumes from multiple protocols..."
require_endpoint "order-service" "api-gateway" "main" "/orders" "POST"
require_grpc     "order-service" "inventory-service" "main" "InventoryService" "CheckStock"
require_asyncapi "order-service" "event-bus" "main" "order-events" "PUB"

# Analytics Service consumes all events
echo "  Analytics Service consumes all events from Event Bus..."
require_asyncapi "analytics-service" "event-bus" "main" "user-signups" "PUB"
require_asyncapi "analytics-service" "event-bus" "main" "order-events" "PUB"

section "3. Verify Reports"

echo ">>> Fetching Markdown Report..."
RESPONSE=$(curl -s -w "\n%{http_code}" "$BASE_URL/report/markdown?branch=main" \
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
