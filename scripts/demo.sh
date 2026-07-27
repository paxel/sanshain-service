#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Extended Demo Script
#
# Demonstrates:
#   1. Multiple services providing OpenAPI specs (main + feature branches)
#   2. Client dependencies (services acting as both provider and consumer)
#   3. Circular dependencies between services
#   4. Backward-compatible updates on protected branches
#   5. Breaking change rejection on protected branches
#   6. Endpoint version history
#   7. Require-bundle (multi-endpoint fetch)
#   8. Dry-run mode
#   9. Dependency report
#  10. Feature-branch fallback to main
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
provide() {
  local svc="$1" branch="$2" yaml="$3"
  echo ">>> PROVIDE  $svc @ $branch"
  PAYLOAD=$(jq -n --arg s "$svc" --arg b "$branch" --arg y "$yaml" \
    '{producername:$s, branch:$b, openapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
  return 0
}

provide_expect() {
  local svc="$1" branch="$2" yaml="$3" expected="$4"
  echo ">>> PROVIDE  $svc @ $branch  (expect $expected)"
  PAYLOAD=$(jq -n --arg s "$svc" --arg b "$branch" --arg y "$yaml" \
    '{producername:$s, branch:$b, openapi_yaml:$y}')
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  BODY=$(echo "$RESPONSE" | sed '$d')
  STATUS=$(echo "$RESPONSE" | tail -1)
  echo "    HTTP $STATUS"
  if [ "$STATUS" != "$expected" ]; then
    echo "    UNEXPECTED! Expected $expected, got $STATUS"
    echo "$BODY" | head -5 | sed 's/^/    /'
  else
    if [ "$STATUS" = "409" ]; then
      echo "    Rejection reason:"
      echo "$BODY" | head -5 | sed 's/^/      /'
    fi
  fi
  return 0
}

require_endpoint() {
  local client="$1" svc="$2" branch="$3" path="$4" method="$5"
  echo ">>> REQUIRE  $client -> $svc $method $path ($branch)"
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require" \
    --data-urlencode "consumername=$client" \
    --data-urlencode "producername=$svc" \
    --data-urlencode "branch=$branch" \
    --data-urlencode "path=$path" \
    --data-urlencode "method=$method" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  BODY=$(echo "$RESPONSE" | sed '$d')
  STATUS=$(echo "$RESPONSE" | tail -1)
  if [ "$STATUS" = "200" ]; then
    LINES=$(echo "$BODY" | wc -l)
    echo "    HTTP $STATUS — $LINES lines of YAML"
  else
    echo "    HTTP $STATUS"
    echo "$BODY" | head -3 | sed 's/^/    /'
  fi
}

require_bundle() {
  local client="$1" branch="$2" svc="$3"; shift 3
  echo ">>> REQUIRE-BUNDLE  $client @ $branch (service: $svc)"
  # remaining args are "path:method" pairs
  local endpoints="[]"
  for pair in "$@"; do
    IFS=: read -r p m <<< "$pair"
    endpoints=$(echo "$endpoints" | jq --arg p "$p" --arg m "$m" \
      '. + [{path:$p, method:$m}]')
  done
  PAYLOAD=$(jq -n --arg c "$client" --arg s "$svc" --arg b "$branch" --argjson e "$endpoints" \
    '{consumername:$c, producername:$s, branch:$b, endpoints:$e}')
  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -X POST "$BASE_URL/require-bundle" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  BODY=$(echo "$RESPONSE" | sed '$d')
  STATUS=$(echo "$RESPONSE" | tail -1)
  LINES=$(echo "$BODY" | wc -l)
  echo "    HTTP $STATUS — $LINES lines of merged YAML"
}

section() {
  echo ""
  echo "==========================================================================="
  echo "  $1"
  echo "==========================================================================="
  echo ""
}

# ============================================================================
# OpenAPI specs for demo services
# ============================================================================

USER_SERVICE_V1='openapi: 3.0.0
info:
  title: User Service
  version: 1.0.0
paths:
  /users:
    get:
      summary: List users
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/User"
    post:
      summary: Create user
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreateUser"
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
  /users/{id}:
    get:
      summary: Get user by ID
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
components:
  schemas:
    User:
      type: object
      properties:
        id:
          type: string
        name:
          type: string
        email:
          type: string
    CreateUser:
      type: object
      properties:
        name:
          type: string
        email:
          type: string'

# V2: adds optional "role" field — backward-compatible
USER_SERVICE_V2='openapi: 3.0.0
info:
  title: User Service
  version: 1.2.0
paths:
  /users:
    get:
      summary: List users
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/User"
    post:
      summary: Create user
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreateUser"
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
  /users/{id}:
    get:
      summary: Get user by ID
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
components:
  schemas:
    User:
      type: object
      properties:
        id:
          type: string
        name:
          type: string
        email:
          type: string
        role:
          type: string
    CreateUser:
      type: object
      properties:
        name:
          type: string
        email:
          type: string
        role:
          type: string'

# V3 (breaking): changes User.id from string to integer
USER_SERVICE_BREAKING='openapi: 3.0.0
info:
  title: User Service
  version: 2.0.0
paths:
  /users:
    get:
      summary: List users
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/User"
    post:
      summary: Create user
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreateUser"
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
  /users/{id}:
    get:
      summary: Get user by ID
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/User"
components:
  schemas:
    User:
      type: object
      properties:
        id:
          type: integer
        name:
          type: string
        email:
          type: string
        role:
          type: string
    CreateUser:
      type: object
      properties:
        name:
          type: string
        email:
          type: string
        role:
          type: string'

ORDER_SERVICE='openapi: 3.0.0
info:
  title: Order Service
  version: 1.0.0
paths:
  /orders:
    get:
      summary: List orders
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Order"
    post:
      summary: Create order
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreateOrder"
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Order"
  /orders/{id}:
    get:
      summary: Get order by ID
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Order"
components:
  schemas:
    Order:
      type: object
      properties:
        id:
          type: string
        userId:
          type: string
        items:
          type: array
          items:
            $ref: "#/components/schemas/OrderItem"
        total:
          type: number
    OrderItem:
      type: object
      properties:
        productId:
          type: string
        quantity:
          type: integer
        price:
          type: number
    CreateOrder:
      type: object
      properties:
        userId:
          type: string
        items:
          type: array
          items:
            $ref: "#/components/schemas/OrderItem"'

ORDER_SERVICE_FEATURE='openapi: 3.0.0
info:
  title: Order Service
  version: 1.0.0-discounts
paths:
  /orders:
    get:
      summary: List orders
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Order"
    post:
      summary: Create order
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreateOrder"
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Order"
  /orders/{id}:
    get:
      summary: Get order by ID
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Order"
  /discounts:
    get:
      summary: List available discounts
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Discount"
components:
  schemas:
    Order:
      type: object
      properties:
        id:
          type: string
        userId:
          type: string
        items:
          type: array
          items:
            $ref: "#/components/schemas/OrderItem"
        total:
          type: number
        discountCode:
          type: string
    OrderItem:
      type: object
      properties:
        productId:
          type: string
        quantity:
          type: integer
        price:
          type: number
    CreateOrder:
      type: object
      properties:
        userId:
          type: string
        items:
          type: array
          items:
            $ref: "#/components/schemas/OrderItem"
        discountCode:
          type: string
    Discount:
      type: object
      properties:
        code:
          type: string
        percentage:
          type: number
        validUntil:
          type: string'

NOTIFICATION_SERVICE='openapi: 3.0.0
info:
  title: Notification Service
  version: 1.0.0
paths:
  /notifications/send:
    post:
      summary: Send notification
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/SendNotification"
      responses:
        "202":
          description: Accepted
  /notifications/user/{userId}:
    get:
      summary: Get notifications for user
      parameters:
        - name: userId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Notification"
components:
  schemas:
    SendNotification:
      type: object
      properties:
        userId:
          type: string
        type:
          type: string
        message:
          type: string
    Notification:
      type: object
      properties:
        id:
          type: string
        userId:
          type: string
        type:
          type: string
        message:
          type: string
        sentAt:
          type: string'

PAYMENT_SERVICE='openapi: 3.0.0
info:
  title: Payment Service
  version: 1.0.0
paths:
  /payments:
    post:
      summary: Process payment
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/PaymentRequest"
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/PaymentResult"
  /payments/{id}:
    get:
      summary: Get payment status
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/PaymentResult"
  /payments/refund:
    post:
      summary: Refund a payment
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/RefundRequest"
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/PaymentResult"
components:
  schemas:
    PaymentRequest:
      type: object
      properties:
        orderId:
          type: string
        amount:
          type: number
        currency:
          type: string
        method:
          type: string
    PaymentResult:
      type: object
      properties:
        id:
          type: string
        orderId:
          type: string
        status:
          type: string
        amount:
          type: number
    RefundRequest:
      type: object
      properties:
        paymentId:
          type: string
        reason:
          type: string'

# inventory-service: used by order-service, also calls notification-service (cycle potential)
INVENTORY_SERVICE='openapi: 3.0.0
info:
  title: Inventory Service
  version: 1.0.0
paths:
  /inventory/{productId}:
    get:
      summary: Check stock level
      parameters:
        - name: productId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/StockLevel"
  /inventory/reserve:
    post:
      summary: Reserve stock for an order
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/ReserveRequest"
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Reservation"
components:
  schemas:
    StockLevel:
      type: object
      properties:
        productId:
          type: string
        available:
          type: integer
        reserved:
          type: integer
    ReserveRequest:
      type: object
      properties:
        orderId:
          type: string
        productId:
          type: string
        quantity:
          type: integer
    Reservation:
      type: object
      properties:
        id:
          type: string
        orderId:
          type: string
        productId:
          type: string
        quantity:
          type: integer
        status:
          type: string'

# ============================================================================
# DEMO START
# ============================================================================

echo "Sanshain Service Demo"
echo "Target: $BASE_URL"
echo ""

# --------------------------------------------------------------------------
section "1. Provide services on main branch"
# --------------------------------------------------------------------------

provide "user-service"         "main" "$USER_SERVICE_V1"
provide "order-service"        "main" "$ORDER_SERVICE"
provide "notification-service" "main" "$NOTIFICATION_SERVICE"
provide "payment-service"      "main" "$PAYMENT_SERVICE"
provide "inventory-service"    "main" "$INVENTORY_SERVICE"

# --------------------------------------------------------------------------
section "2. Provide feature branches"
# --------------------------------------------------------------------------

provide "order-service"  "feature/discounts"  "$ORDER_SERVICE_FEATURE"
provide "user-service"   "feature/add-roles"  "$USER_SERVICE_V2"

# --------------------------------------------------------------------------
section "3. Register client dependencies — services as both providers & consumers"
# --------------------------------------------------------------------------

# order-service depends on user-service (to look up users)
require_endpoint "order-service" "user-service" "main" "/users/{id}" "GET"

# order-service depends on inventory-service (to reserve stock)
require_endpoint "order-service" "inventory-service" "main" "/inventory/reserve" "POST"

# order-service depends on payment-service (to process payments)
require_endpoint "order-service" "payment-service" "main" "/payments" "POST"

# notification-service depends on user-service (to look up user contact info)
require_endpoint "notification-service" "user-service" "main" "/users/{id}" "GET"

# payment-service depends on order-service (to verify order details)
require_endpoint "payment-service" "order-service" "main" "/orders/{id}" "GET"

# payment-service depends on notification-service (to send payment confirmations)
require_endpoint "payment-service" "notification-service" "main" "/notifications/send" "POST"

echo ""
echo "  At this point, order-service -> payment-service -> order-service"
echo "  forms a circular dependency. The graph view will highlight this."

# --------------------------------------------------------------------------
section "4. Circular dependency: inventory-service -> order-service -> inventory-service"
# --------------------------------------------------------------------------

# inventory-service depends on order-service (to check order status for reservations)
require_endpoint "inventory-service" "order-service" "main" "/orders/{id}" "GET"

# inventory-service also depends on notification-service (low-stock alerts)
require_endpoint "inventory-service" "notification-service" "main" "/notifications/send" "POST"

echo ""
echo "  Now we have two cycles:"
echo "    order-service -> inventory-service -> order-service"
echo "    order-service -> payment-service -> order-service"

# --------------------------------------------------------------------------
section "5. External consumers (web-frontend, mobile-app)"
# --------------------------------------------------------------------------

require_endpoint "web-frontend" "user-service"         "main" "/users"     "GET"
require_endpoint "web-frontend" "user-service"         "main" "/users"     "POST"
require_endpoint "web-frontend" "order-service"        "main" "/orders"    "GET"
require_endpoint "web-frontend" "order-service"        "main" "/orders"    "POST"
require_endpoint "web-frontend" "notification-service" "main" "/notifications/user/{userId}" "GET"

require_endpoint "mobile-app" "user-service"    "main" "/users/{id}" "GET"
require_endpoint "mobile-app" "order-service"   "main" "/orders"     "POST"
require_endpoint "mobile-app" "payment-service" "main" "/payments/{id}" "GET"

# --------------------------------------------------------------------------
section "6. Feature-branch dependencies"
# --------------------------------------------------------------------------

# ci-pipeline tests the discounts feature
require_endpoint "ci-pipeline" "order-service" "feature/discounts" "/discounts" "GET"
require_endpoint "ci-pipeline" "order-service" "feature/discounts" "/orders"    "POST"

# --------------------------------------------------------------------------
section "7. Backward-compatible update on protected branch"
# --------------------------------------------------------------------------

echo "  Updating user-service on main with a new optional 'role' field..."
provide_expect "user-service" "main" "$USER_SERVICE_V2" "202"

# --------------------------------------------------------------------------
section "8. Breaking change rejection on protected branch"
# --------------------------------------------------------------------------

echo "  Attempting to change User.id from string to integer (breaking!)..."
provide_expect "user-service" "main" "$USER_SERVICE_BREAKING" "409"

# --------------------------------------------------------------------------
section "9. Endpoint version history"
# --------------------------------------------------------------------------

echo ">>> Fetching version history for user-service GET /users on main..."
RESPONSE=$(curl -s -w "\n%{http_code}" \
  -G "$BASE_URL/endpoint-versions" \
  --data-urlencode "service=user-service" \
  --data-urlencode "branch=main" \
  --data-urlencode "path=/users" \
  --data-urlencode "method=GET" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
BODY=$(echo "$RESPONSE" | sed '$d')
STATUS=$(echo "$RESPONSE" | tail -1)
echo "    HTTP $STATUS"
VERSIONS=$(echo "$BODY" | jq 'length' 2>/dev/null || echo "?")
echo "    Versions recorded: $VERSIONS"
if [ "$VERSIONS" != "?" ] && [ "$VERSIONS" -gt 0 ]; then
  echo "    Latest version diff preview:"
  echo "$BODY" | jq -r '.[-1].diff_from_previous // "initial version"' | head -10 | sed 's/^/      /'
fi

# --------------------------------------------------------------------------
section "10. Require-bundle (multi-endpoint fetch)"
# --------------------------------------------------------------------------

require_bundle "web-frontend" "main" "user-service" \
  "/users:GET"
require_bundle "web-frontend" "main" "order-service" \
  "/orders:GET"
require_bundle "web-frontend" "main" "payment-service" \
  "/payments/{id}:GET"

# --------------------------------------------------------------------------
section "11. Dry-run mode"
# --------------------------------------------------------------------------

echo ">>> DRY-RUN provide (validate without persisting)..."
PAYLOAD=$(jq -n --arg s "dry-run-test" --arg b "main" --arg y "$USER_SERVICE_V1" \
  '{producername:$s, branch:$b, openapi_yaml:$y, dry_run:true}')
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$BASE_URL/provide" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
  -d "$PAYLOAD")
echo "    HTTP $STATUS (service should NOT appear in reports)"

echo ""
echo ">>> DRY-RUN require (validate without recording dependency)..."
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -G "$BASE_URL/require" \
  --data-urlencode "consumername=dry-run-client" \
  --data-urlencode "producername=user-service" \
  --data-urlencode "branch=main" \
  --data-urlencode "path=/users" \
  --data-urlencode "method=GET" \
  --data-urlencode "dry_run=true" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
echo "    HTTP $STATUS (dependency should NOT appear in graph)"

# --------------------------------------------------------------------------
section "12. Dependency report"
# --------------------------------------------------------------------------

echo ">>> Fetching dependency report (JSON)..."
RESPONSE=$(curl -s -w "\n%{http_code}" "$BASE_URL/report?branch=main" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
BODY=$(echo "$RESPONSE" | sed '$d')
STATUS=$(echo "$RESPONSE" | tail -1)
SERVICES=$(echo "$BODY" | jq '.services | length' 2>/dev/null || echo "?")
CLIENTS=$(echo "$BODY" | jq '.clients | length' 2>/dev/null || echo "?")
echo "    HTTP $STATUS — $SERVICES services, $CLIENTS clients"

echo ""
echo ">>> Fetching dependency report (Markdown)..."
RESPONSE=$(curl -s -w "\n%{http_code}" "$BASE_URL/report/markdown?branch=main" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
BODY=$(echo "$RESPONSE" | sed '$d')
STATUS=$(echo "$RESPONSE" | tail -1)
LINES=$(echo "$BODY" | wc -l)
echo "    HTTP $STATUS — $LINES lines of Markdown"
echo "    Preview:"
echo "$BODY" | head -15 | sed 's/^/      /'

# --------------------------------------------------------------------------
section "13. Feature-branch fallback to main"
# --------------------------------------------------------------------------

echo "  Requesting an endpoint from a non-existent feature branch..."
echo "  Should fall back to main branch."
require_endpoint "fallback-client" "user-service" "feature/nonexistent" "/users" "GET"

# ============================================================================
echo ""
echo "==========================================================================="
echo "  Demo complete!"
echo ""
echo "  Open $BASE_URL to explore:"
echo "    - Service overview with drill-down to endpoints and YAML"
echo "    - Client overview showing which services each client depends on"
echo "    - Dependency graph with cycle detection (red edges)"
echo "    - Admin dashboard for branch and service management"
echo "==========================================================================="
