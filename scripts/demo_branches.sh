#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Demo 4: Release Graphs (ADR-0004/0005)
#
# A small web-shop scenario exercising the trunk stream and sanshain-branches,
# sized to keep the Main graph view readable:
#   1. Producers provide with `trunk: true` — the trunk stream's versions
#   2. Consumers require with `trunk=true` — the main graph's pin set
#   3. A major bump the consumer hasn't followed (red conflict edge)
#   4. A snapshot-pinned trunk dependency (snapshot highlight)
#   5. A release cut: POST /admin/branches ("release-maribou")
#   6. A branch hotfix via tag-provide/require (branch differs from main)
#   7. A deleted version under a pin (dangling reference, orange)
# ============================================================================

BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
ADMIN_USER="${SANSHAIN_USER:-root}"
ADMIN_PASSWORD="${SANSHAIN_PASSWORD:-}"
TOKEN="${SANSHAIN_TOKEN:-}"
BRANCH_NAME="${SANSHAIN_DEMO_BRANCH:-release-maribou}"

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

# provide_stream <svc> <stability> <yaml> [trunk|tag=<branch>]
provide_stream() {
  local svc="$1" stability="$2" yaml="$3" stream="${4:-}"
  echo ">>> PROVIDE  $svc ($stability${stream:+, $stream})"
  local filter='{producername:$s, stability:$st, openapi_yaml:$y}'
  case "$stream" in
    trunk) filter='{producername:$s, stability:$st, openapi_yaml:$y, trunk:true}' ;;
    tag=*) filter="{producername:\$s, stability:\$st, openapi_yaml:\$y, tag:\"${stream#tag=}\"}" ;;
  esac
  PAYLOAD=$(jq -n --arg s "$svc" --arg st "$stability" --arg y "$yaml" "$filter")
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/provide" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "$PAYLOAD")
  echo "    HTTP $STATUS"
}

# require_stream <client> <svc> <version> <path> <method> [trunk|tag=<branch>]
require_stream() {
  local client="$1" svc="$2" version="$3" path="$4" method="$5" stream="${6:-}"
  echo ">>> REQUIRE  $client -> $svc $method $path @ $version${stream:+ ($stream)}"
  local extra=()
  case "$stream" in
    trunk) extra=(--data-urlencode "trunk=true") ;;
    tag=*) extra=(--data-urlencode "tag=${stream#tag=}") ;;
  esac
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -G "$BASE_URL/require" \
    --data-urlencode "consumername=$client" \
    --data-urlencode "producername=$svc" \
    --data-urlencode "version=$version" \
    --data-urlencode "path=$path" \
    --data-urlencode "method=$method" \
    "${extra[@]}" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"})
  echo "    HTTP $STATUS"
}

spec() {
  local title="$1" version="$2" path="$3" method="$4"
  cat <<EOF
openapi: 3.0.3
info:
  title: $title
  version: $version
paths:
  $path:
    ${method,,}:
      summary: $title
      responses:
        '200':
          description: OK
EOF
}

# ---------------------------------------------------------------------------
# 1+2. The trunk stream: producers at their trunk version, consumers pinned
# ---------------------------------------------------------------------------
echo ""
echo "=== Release graphs: trunk stream ==="
provide_stream payment-gateway  ga       "$(spec 'Payment Gateway' 1.0.0 /payments POST)" trunk
provide_stream inventory-api    ga       "$(spec 'Inventory API'   1.2.0 /stock GET)"     trunk
provide_stream notification-hub ga       "$(spec 'Notification Hub' 1.0.0 /notify POST)"  trunk
provide_stream shipping-service snapshot "$(spec 'Shipping Service' 0.9.0 /ship POST)"    trunk

require_stream checkout-service payment-gateway  1.0.0 /payments POST trunk
require_stream checkout-service inventory-api    1.2.0 /stock    GET  trunk
require_stream checkout-service notification-hub 1.0.0 /notify   POST trunk
require_stream checkout-service shipping-service 0.9.0 /ship     POST trunk
require_stream storefront-web   inventory-api    1.2.0 /stock    GET  trunk

# ---------------------------------------------------------------------------
# 3. Trunk moves a major ahead of the pin: red conflict edge in the Main view
# ---------------------------------------------------------------------------
echo ""
echo "=== Trunk major bump the consumer has not followed ==="
provide_stream payment-gateway ga "$(spec 'Payment Gateway' 2.0.0 /payments POST)" trunk

# ---------------------------------------------------------------------------
# 5. The release cut: a sanshain-branch copying the current main graph
# ---------------------------------------------------------------------------
echo ""
echo "=== Release cut: sanshain-branch '$BRANCH_NAME' ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$BASE_URL/admin/branches" \
  -H "Content-Type: application/json" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
  -d "{\"name\":\"$BRANCH_NAME\"}")
echo "    HTTP $STATUS"

# ---------------------------------------------------------------------------
# 6. A hotfix lands on the branch only (tag-provide + tag-require)
# ---------------------------------------------------------------------------
echo ""
echo "=== Branch hotfix via tag=$BRANCH_NAME ==="
provide_stream notification-hub ga "$(spec 'Notification Hub' 1.0.1 /notify POST)" "tag=$BRANCH_NAME"
require_stream checkout-service notification-hub 1.0.1 /notify POST "tag=$BRANCH_NAME"

# ---------------------------------------------------------------------------
# 7. A version deleted under a pin: dangling reference in every graph view
# ---------------------------------------------------------------------------
echo ""
echo "=== Delete shipping-service 0.9.0 (dangling pin) ==="
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X DELETE "$BASE_URL/admin/producers/shipping-service/versions/openapi/0.9.0" \
  ${AUTH_HEADER:+-H "$AUTH_HEADER"})
echo "    HTTP $STATUS"

echo ""
echo "Release-graph demo data complete: switch the graph page to Main, pick"
echo "'$BRANCH_NAME' from the branch selector, or diff the two."
