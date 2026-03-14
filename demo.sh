#!/usr/bin/env bash
set -euo pipefail

# Configuration
BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
TOKEN="${SANSHAIN_TOKEN:-}"  # set to your san_... API token or session token

AUTH_HEADER=""
if [ -n "$TOKEN" ]; then
  AUTH_HEADER="Authorization: Bearer $TOKEN"
fi

# ---------------------------------------------------------------------------
# Helper: provide an OpenAPI spec for a service/branch
# ---------------------------------------------------------------------------
provide() {
  local svc="$1" branch="$2" yaml_file="$3"

  echo ">>> Providing '$yaml_file' as service '$svc' on branch '$branch'..."

  YAML_CONTENT=$(cat "$yaml_file")

  PAYLOAD=$(jq -n \
    --arg svc "$svc" \
    --arg br "$branch" \
    --arg yaml "$YAML_CONTENT" \
    '{servicename: $svc, branch: $br, openapi_yaml: $yaml}')

  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")

  echo "    HTTP $STATUS"
  if [ "$STATUS" != "202" ]; then
    echo "    ERROR: Expected 202, got $STATUS"
    exit 1
  fi
}

# ---------------------------------------------------------------------------
# Helper: require an endpoint and register a client dependency
# ---------------------------------------------------------------------------
require_endpoint() {
  local client="$1" svc="$2" branch="$3" path="$4" method="$5"

  echo ">>> '$client' requires $method $path from '$svc' ($branch)..."

  RESPONSE=$(curl -s -w "\n%{http_code}" \
    -G "$BASE_URL/require" \
    --data-urlencode "clientname=$client" \
    --data-urlencode "servicename=$svc" \
    --data-urlencode "branch=$branch" \
    --data-urlencode "path=$path" \
    --data-urlencode "method=$method" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})

  BODY=$(echo "$RESPONSE" | sed '$d')
  STATUS=$(echo "$RESPONSE" | tail -1)

  if [ "$STATUS" = "200" ]; then
    LINES=$(echo "$BODY" | wc -l)
    echo "    HTTP $STATUS — received $LINES lines of YAML"
  else
    echo "    ERROR: HTTP $STATUS"
    echo "$BODY" | head -5 | sed 's/^/    /'
    exit 1
  fi
}

# ===========================================================================
# 1. Provide services
# ===========================================================================
echo "=== Providing services ==="
echo ""

provide "sanshain-client-api" "main"    api.yaml
provide "sanshain-client-api" "feature/new-auth" api.yaml
provide "user-service"        "main"    api.yaml
provide "order-service"       "main"    api.yaml
provide "order-service"       "feature/discounts" api.yaml

echo ""

# ===========================================================================
# 2. Register client dependencies
# ===========================================================================
echo "=== Registering client dependencies ==="
echo ""

# web-frontend depends on several services
require_endpoint "web-frontend" "sanshain-client-api" "main" "/provide" "POST"
require_endpoint "web-frontend" "sanshain-client-api" "main" "/require" "GET"
require_endpoint "web-frontend" "user-service"        "main" "/provide" "POST"
require_endpoint "web-frontend" "order-service"       "main" "/provide" "POST"

# mobile-app depends on user-service and order-service
require_endpoint "mobile-app" "user-service"  "main" "/provide" "POST"
require_endpoint "mobile-app" "order-service" "main" "/require" "GET"

# ci-pipeline depends on sanshain-client-api on a feature branch
require_endpoint "ci-pipeline" "sanshain-client-api" "feature/new-auth" "/provide" "POST"

# analytics-worker depends on order-service feature branch
require_endpoint "analytics-worker" "order-service" "feature/discounts" "/provide" "POST"

echo ""
echo "=== Done — open $BASE_URL to explore the dependency graph ==="
