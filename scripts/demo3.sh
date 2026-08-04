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
# All services provide as GA under version 1.0.0 (read from each spec's
# info.version) and every dependency pins that version, so the scenario sits
# alongside the demo / demo2 data in the same dependency graph.
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
PIN="1.0.0"

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
  local svc="$1" stability="$2" yaml="$3"
  echo ">>> PROVIDE  $svc ($stability)"
  PAYLOAD=$(jq -n --arg s "$svc" --arg st "$stability" --arg y "$yaml" \
    '{producername:$s, stability:$st, openapi_yaml:$y}')
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
  return 0
}

require_endpoint() {
  local client="$1" svc="$2" version="$3" path="$4" method="$5"
  echo ">>> REQUIRE  $client -> $svc $method $path @ $version"
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

section "Demo 3 — Google Cloud Online Boutique (all pins @ $PIN)"
echo "Source: https://github.com/GoogleCloudPlatform/microservices-demo"

section "1. Providing Services (GA $PIN)"

provide "frontend"                "ga" "$(GENERIC_SPEC 'Frontend (HTTP UI)')"
provide "cartservice"             "ga" "$CART_SPEC"
provide "productcatalogservice"   "ga" "$(GENERIC_SPEC 'Product Catalog Service')"
provide "currencyservice"         "ga" "$(GENERIC_SPEC 'Currency Service')"
provide "paymentservice"          "ga" "$(GENERIC_SPEC 'Payment Service')"
provide "shippingservice"         "ga" "$(GENERIC_SPEC 'Shipping Service')"
provide "emailservice"            "ga" "$(GENERIC_SPEC 'Email Service')"
provide "checkoutservice"         "ga" "$CHECKOUT_SPEC"
provide "recommendationservice"   "ga" "$(GENERIC_SPEC 'Recommendation Service')"
provide "adservice"               "ga" "$(GENERIC_SPEC 'Ad Service')"
provide "loadgenerator"           "ga" "$(GENERIC_SPEC 'Load Generator (Locust)')"

section "2. Registering Dependencies (per upstream architecture diagram)"

echo ">>> frontend -> downstream services"
require_endpoint "frontend" "productcatalogservice" "$PIN" "/api/v1/resource" "GET"
require_endpoint "frontend" "cartservice"           "$PIN" "/cart/{userId}"   "GET"
require_endpoint "frontend" "currencyservice"       "$PIN" "/api/v1/resource" "GET"
require_endpoint "frontend" "recommendationservice" "$PIN" "/api/v1/resource" "GET"
require_endpoint "frontend" "shippingservice"       "$PIN" "/api/v1/resource" "GET"
require_endpoint "frontend" "checkoutservice"       "$PIN" "/checkout"        "POST"
require_endpoint "frontend" "adservice"             "$PIN" "/api/v1/resource" "GET"

echo ">>> checkoutservice -> downstream services"
require_endpoint "checkoutservice" "cartservice"           "$PIN" "/cart/{userId}"   "GET"
require_endpoint "checkoutservice" "productcatalogservice" "$PIN" "/api/v1/resource" "GET"
require_endpoint "checkoutservice" "currencyservice"       "$PIN" "/api/v1/resource" "GET"
require_endpoint "checkoutservice" "paymentservice"        "$PIN" "/api/v1/resource" "POST"
require_endpoint "checkoutservice" "shippingservice"       "$PIN" "/api/v1/resource" "POST"
require_endpoint "checkoutservice" "emailservice"          "$PIN" "/api/v1/resource" "POST"

echo ">>> recommendationservice -> productcatalogservice"
require_endpoint "recommendationservice" "productcatalogservice" "$PIN" "/api/v1/resource" "GET"

echo ">>> loadgenerator -> frontend (Locust traffic)"
require_endpoint "loadgenerator" "frontend" "$PIN" "/api/v1/resource" "GET"

echo ""
echo "==========================================================================="
echo "  Demo 3 Scenario complete!"
echo "  Registered 11 Online Boutique services, all pinned to $PIN."
echo "  Open $BASE_URL to see them in the dependency graph."
echo "  Source: https://github.com/GoogleCloudPlatform/microservices-demo"
echo "==========================================================================="
