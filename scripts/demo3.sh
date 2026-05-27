#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Demo 3: Google Cloud "Online Boutique" (Hipster Shop)
#
# Public source:
#   https://github.com/GoogleCloudPlatform/microservices-demo
#   Architecture diagram:
#   https://github.com/GoogleCloudPlatform/microservices-demo/blob/main/docs/img/architecture-diagram.png
#
# Online Boutique is a cloud-first microservices demo application
# open-sourced by Google. It consists of an 11-service polyglot e-commerce
# architecture used throughout Google's documentation, conference talks and
# training material (Kubernetes, Istio, Anthos Service Mesh).
#
# All services are registered on the dedicated "google" branch so that the
# dependency graph in the Sanshain UI can be viewed by switching to that
# branch alongside the existing `main` (demo / demo2) scenario.
#
# Services (11):
#   frontend, cartservice, productcatalogservice, currencyservice,
#   paymentservice, shippingservice, emailservice, checkoutservice,
#   recommendationservice, adservice, loadgenerator
#
# Dependencies reproduced from the upstream architecture diagram.
# ============================================================================

BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
ADMIN_USER="${SANSHAIN_USER:-root}"
ADMIN_PASSWORD="${SANSHAIN_PASSWORD:-}"
TOKEN="${SANSHAIN_TOKEN:-}"
BRANCH="${DEMO3_BRANCH:-google}"

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
# Helpers (same CLI-registration style as demo.sh / demo2.sh)
# ---------------------------------------------------------------------------
provide() {
  local svc="$1" branch="$2" yaml="$3"
  echo ">>> PROVIDE  $svc @ $branch"
  PAYLOAD=$(jq -n --arg s "$svc" --arg b "$branch" --arg y "$yaml" \
    '{servicename:$s, branch:$b, openapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
  return 0
}

require_endpoint() {
  local client="$1" svc="$2" branch="$3" path="$4" method="$5"
  echo ">>> REQUIRE  $client -> $svc $method $path ($branch)"
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

section() {
  echo ""
  echo "==========================================================================="
  echo "  $1"
  echo "==========================================================================="
  echo ""
}

# ---------------------------------------------------------------------------
# OpenAPI Specs (minimal, modelled after the upstream gRPC service contracts
# documented in GoogleCloudPlatform/microservices-demo/protos/demo.proto)
# ---------------------------------------------------------------------------
GENERIC_SPEC() {
  local title="$1"
  echo "openapi: 3.0.0
info:
  title: $title
  version: 1.0.0
paths:
  /health:
    get:
      summary: Health check
      responses:
        '200':
          description: OK
  /api/v1/resource:
    get:
      summary: Get resource
      responses:
        '200':
          description: OK
    post:
      summary: Create resource
      responses:
        '201':
          description: Created"
}

CART_SPEC='openapi: 3.0.0
info:
  title: Cart Service
  version: 1.0.0
paths:
  /cart/{userId}:
    get:
      summary: Get cart
      responses:
        "200":
          description: OK
  /cart:
    post:
      summary: Add item
      responses:
        "201":
          description: Added
    delete:
      summary: Empty cart
      responses:
        "204":
          description: Emptied'

CHECKOUT_SPEC='openapi: 3.0.0
info:
  title: Checkout Service
  version: 1.0.0
paths:
  /checkout:
    post:
      summary: Place order
      responses:
        "200":
          description: Order placed'

# ============================================================================
# DEMO START
# ============================================================================

section "Demo 3 — Google Cloud Online Boutique (branch: $BRANCH)"
echo "Source: https://github.com/GoogleCloudPlatform/microservices-demo"

section "1. Providing Services"

provide "frontend"                "$BRANCH" "$(GENERIC_SPEC 'Frontend (HTTP UI)')"
provide "cartservice"             "$BRANCH" "$CART_SPEC"
provide "productcatalogservice"   "$BRANCH" "$(GENERIC_SPEC 'Product Catalog Service')"
provide "currencyservice"         "$BRANCH" "$(GENERIC_SPEC 'Currency Service')"
provide "paymentservice"          "$BRANCH" "$(GENERIC_SPEC 'Payment Service')"
provide "shippingservice"         "$BRANCH" "$(GENERIC_SPEC 'Shipping Service')"
provide "emailservice"            "$BRANCH" "$(GENERIC_SPEC 'Email Service')"
provide "checkoutservice"         "$BRANCH" "$CHECKOUT_SPEC"
provide "recommendationservice"   "$BRANCH" "$(GENERIC_SPEC 'Recommendation Service')"
provide "adservice"               "$BRANCH" "$(GENERIC_SPEC 'Ad Service')"
provide "loadgenerator"           "$BRANCH" "$(GENERIC_SPEC 'Load Generator (Locust)')"

section "2. Registering Dependencies (per upstream architecture diagram)"

echo ">>> frontend -> downstream services"
require_endpoint "frontend" "productcatalogservice" "$BRANCH" "/api/v1/resource" "GET"
require_endpoint "frontend" "cartservice"           "$BRANCH" "/cart/{userId}"   "GET"
require_endpoint "frontend" "currencyservice"       "$BRANCH" "/api/v1/resource" "GET"
require_endpoint "frontend" "recommendationservice" "$BRANCH" "/api/v1/resource" "GET"
require_endpoint "frontend" "shippingservice"       "$BRANCH" "/api/v1/resource" "GET"
require_endpoint "frontend" "checkoutservice"       "$BRANCH" "/checkout"        "POST"
require_endpoint "frontend" "adservice"             "$BRANCH" "/api/v1/resource" "GET"

echo ">>> checkoutservice -> downstream services"
require_endpoint "checkoutservice" "cartservice"           "$BRANCH" "/cart/{userId}"   "GET"
require_endpoint "checkoutservice" "productcatalogservice" "$BRANCH" "/api/v1/resource" "GET"
require_endpoint "checkoutservice" "currencyservice"       "$BRANCH" "/api/v1/resource" "GET"
require_endpoint "checkoutservice" "paymentservice"        "$BRANCH" "/api/v1/resource" "POST"
require_endpoint "checkoutservice" "shippingservice"       "$BRANCH" "/api/v1/resource" "POST"
require_endpoint "checkoutservice" "emailservice"          "$BRANCH" "/api/v1/resource" "POST"

echo ">>> recommendationservice -> productcatalogservice"
require_endpoint "recommendationservice" "productcatalogservice" "$BRANCH" "/api/v1/resource" "GET"

echo ">>> loadgenerator -> frontend (Locust traffic)"
require_endpoint "loadgenerator" "frontend" "$BRANCH" "/api/v1/resource" "GET"

echo ""
echo "==========================================================================="
echo "  Demo 3 Scenario complete!"
echo "  Registered 11 Online Boutique services on branch '$BRANCH'."
echo "  In the Sanshain UI, switch to the '$BRANCH' branch to view the graph."
echo "  Source: https://github.com/GoogleCloudPlatform/microservices-demo"
echo "==========================================================================="
