#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — format=openapi end-to-end check (manual, not CI)
#
# Provides an AsyncAPI 2.6 and an AsyncAPI 3.0 spec to a running service,
# requires their channels with `format=openapi`, and runs openapi-generator
# 7.x (generator `spring`, models only — the CLI form of the Maven plugin's
# `generateApis=false`) on each answer. Fails when the service rejects a
# step, the generator reports an error, or no model class is generated.
# Only published (PUB) channels are provided endpoints a Consumer can require;
# subscriptions are harvested, not served (ADR-0006).
#
# Usage:
#   ./scripts/openapi-generator-check.sh [base_url] [admin_password]
#
# Default base_url: http://localhost:3000
# Default admin_password: INITIAL_ADMIN_PASSWORD env var
# Needs: curl, jq, java 11+ (JAVA_HOME or java on PATH).
# The generator jar is downloaded once from Maven Central into
# ${XDG_CACHE_HOME:-~/.cache}/sanshain/.
# ============================================================================

BASE_URL="${1:-${SANSHAIN_URL:-http://localhost:3000}}"
ADMIN_PASSWORD="${2:-${INITIAL_ADMIN_PASSWORD:-}}"
GENERATOR_VERSION="7.26.0"

if [ -z "$ADMIN_PASSWORD" ]; then
    echo "No admin password provided. Set INITIAL_ADMIN_PASSWORD or pass it as the second argument."
    exit 1
fi

JAVA="${JAVA_HOME:+$JAVA_HOME/bin/}java"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/sanshain"
JAR="$CACHE_DIR/openapi-generator-cli-$GENERATOR_VERSION.jar"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

if [ ! -f "$JAR" ]; then
    mkdir -p "$CACHE_DIR"
    echo "Downloading openapi-generator-cli $GENERATOR_VERSION..."
    curl -fsSL -o "$JAR.part" \
        "https://repo1.maven.org/maven2/org/openapitools/openapi-generator-cli/$GENERATOR_VERSION/openapi-generator-cli-$GENERATOR_VERSION.jar"
    mv "$JAR.part" "$JAR"
fi

TOKEN=$(curl -fsS -X POST "$BASE_URL/auth/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"root\",\"password\":\"$ADMIN_PASSWORD\"}" | jq -r .token)

V2_SPEC=$(cat <<'YAML'
asyncapi: 2.6.0
info:
  title: E2E Events v2
  version: 1.0.0
channels:
  user.created:
    publish:
      message:
        $ref: '#/components/messages/UserCreated'
  order.placed:
    publish:
      message:
        messageId: OrderPlaced
        payload:
          type: object
          required: [order]
          properties:
            order:
              $ref: '#/components/schemas/Order'
            tags:
              type: array
              items:
                type: string
components:
  messages:
    UserCreated:
      payload:
        $ref: '#/components/schemas/User'
  schemas:
    User:
      type: object
      properties:
        id:
          type: string
          format: uuid
        address:
          $ref: '#/components/schemas/Address'
    Address:
      type: object
      properties:
        street:
          type: string
    Order:
      type: object
      properties:
        id:
          type: string
        total:
          type: number
YAML
)

V3_SPEC=$(cat <<'YAML'
asyncapi: 3.0.0
info:
  title: E2E Events v3
  version: 1.0.0
channels:
  userSignedUp:
    address: user/signedup
    messages:
      UserSignedUp:
        $ref: '#/components/messages/UserSignedUp'
  orderShipped:
    address: order/shipped
    messages:
      orderShipped:
        payload:
          type: object
          properties:
            parcel:
              $ref: '#/components/schemas/Parcel'
operations:
  sendUserSignedUp:
    action: send
    channel:
      $ref: '#/channels/userSignedUp'
    messages:
      - $ref: '#/channels/userSignedUp/messages/UserSignedUp'
  sendOrderShipped:
    action: send
    channel:
      $ref: '#/channels/orderShipped'
components:
  messages:
    UserSignedUp:
      payload:
        $ref: '#/components/schemas/User'
  schemas:
    User:
      type: object
      properties:
        id:
          type: string
    Parcel:
      type: object
      properties:
        weight:
          type: number
YAML
)

# check <producer> <spec> <endpoints-json>
check() {
    local producer="$1" spec="$2" endpoints="$3"
    local out="$WORK_DIR/$producer"
    mkdir -p "$out"

    jq -n --arg p "$producer" --arg y "$spec" \
        '{producername: $p, asyncapi_yaml: $y, stability: "snapshot"}' |
        curl -fsS -o /dev/null -X POST "$BASE_URL/provide/asyncapi" \
            -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d @-

    jq -n --arg p "$producer" --argjson e "$endpoints" \
        '{consumername: "e2e-consumer", producername: $p, version: "1.0.0", api_type: "asyncapi", endpoints: $e, dry_run: true}' |
        curl -fsS -o "$out/openapi.yaml" -X POST "$BASE_URL/require-bundle?format=openapi" \
            -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d @-

    grep -q '^openapi: 3.0.3$' "$out/openapi.yaml" || {
        echo "[FAIL] $producer: the answer is not an OpenAPI 3.0.3 document"
        cat "$out/openapi.yaml"
        exit 1
    }

    "$JAVA" -jar "$JAR" generate -g spring -i "$out/openapi.yaml" -o "$out/gen" \
        --global-property models,modelDocs=false,modelTests=false >"$out/generator.log" 2>&1 || {
        echo "[FAIL] $producer: openapi-generator failed"
        cat "$out/generator.log"
        exit 1
    }
    if grep -q "ERROR" "$out/generator.log"; then
        echo "[FAIL] $producer: openapi-generator logged errors"
        cat "$out/generator.log"
        exit 1
    fi

    local models
    models=$(find "$out/gen" -name '*.java' -printf '%f ' | tr -s ' ')
    if [ -z "$models" ]; then
        echo "[FAIL] $producer: no model classes generated"
        exit 1
    fi
    echo "[PASS] $producer: generated $models"
}

check "e2e-openapi-v2" "$V2_SPEC" \
    '[{"path":"user.created","method":"PUB"},{"path":"order.placed","method":"PUB"}]'
check "e2e-openapi-v3" "$V3_SPEC" \
    '[{"path":"user/signedup","method":"PUB"},{"path":"order/shipped","method":"PUB"}]'
