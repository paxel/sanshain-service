#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Sanshain Service — Complex Scenario Demo (30 Services)
#
# Architecture:
# - Frontend-Middleware-Backend chain
# - Complex ETL Pipeline with central orchestration and multiple enrichers
# - ML System (Feature Store, Training, Inference, AB Testing)
# - Multiple Data Sources & Exports
# - Infrastructure wrappers (Postgres, Elastic, Redis, Kafka)
# - Cross-cutting concerns (Auth, Config, Audit, Rules)
# ============================================================================

BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
ADMIN_USER="${SANSHAIN_USER:-root}"
ADMIN_PASSWORD="${SANSHAIN_PASSWORD:-}"
TOKEN="${SANSHAIN_TOKEN:-}"

if [ -z "$TOKEN" ] && [ -n "$ADMIN_PASSWORD" ]; then
  echo ">>> Logging in as $ADMIN_USER to obtain token..."
  LOGIN_RESPONSE=$(curl -s -X POST "$BASE_URL/auth/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"$ADMIN_USER\", \"password\":\"$ADMIN_PASSWORD\"}")
  echo ">>> Login Response: $LOGIN_RESPONSE"
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
  BODY=$(echo "$RESPONSE" | sed '$d')
  STATUS=$(echo "$RESPONSE" | tail -1)
  if [ "$STATUS" = "200" ]; then
    echo "    HTTP $STATUS — Success"
  else
    echo "    HTTP $STATUS"
    echo "$BODY" | head -3 | sed 's/^/    /'
  fi
}

section() {
  echo ""
  echo "==========================================================================="
  echo "  $1"
  echo "==========================================================================="
  echo ""
}

# ---------------------------------------------------------------------------
# OpenAPI Specs
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

ETL_ORCHESTRATOR_SPEC='openapi: 3.0.0
info:
  title: ETL Orchestrator
  version: 1.0.0
paths:
  /health:
    get:
      summary: Health check
      responses:
        "200":
          description: OK
  /jobs:
    post:
      summary: Trigger new ETL job
      responses:
        "202":
          description: Job accepted
    get:
      summary: List jobs
      responses:
        "200":
          description: OK
  /jobs/{id}:
    get:
      summary: Get job status
      responses:
        "200":
          description: OK'

ML_INFERENCE_SPEC='openapi: 3.0.0
info:
  title: ML Inference API
  version: 1.0.0
paths:
  /predict:
    post:
      summary: Run model prediction
      requestBody:
        content:
          application/json:
            schema:
              type: object
      responses:
        "200":
          description: Prediction result'

SEARCH_SPEC='openapi: 3.0.0
info:
  title: Search Wrapper (Elastic)
  version: 1.0.0
paths:
  /search:
    get:
      parameters:
        - name: q
          in: query
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Search results
  /index:
    post:
      summary: Index data
      responses:
        "202":
          description: Indexed'

# ============================================================================
# DEMO START
# ============================================================================

section "1. Providing Infrastructure & Core Services"

provide "config-service"   "main" "$(GENERIC_SPEC 'Config Service')"
provide "secret-manager"   "main" "$(GENERIC_SPEC 'Secret Manager')"
provide "auth-service"     "main" "$(GENERIC_SPEC 'Auth Service')"
provide "audit-service"    "main" "$(GENERIC_SPEC 'Audit Log Service')"
provide "postgres-wrapper" "main" "$(GENERIC_SPEC 'Postgres DB Wrapper')"
provide "redis-wrapper"    "main" "$(GENERIC_SPEC 'Redis Cache Wrapper')"
provide "kafka-wrapper"    "main" "$(GENERIC_SPEC 'Kafka Message Bus')"
provide "elastic-wrapper"  "main" "$SEARCH_SPEC"

section "2. Providing Data Sources & Ingestion"

provide "ingestion-stream-a" "main" "$(GENERIC_SPEC 'Data Source A')"
provide "ingestion-stream-b" "main" "$(GENERIC_SPEC 'Data Source B')"
provide "ingestion-legacy"   "main" "$(GENERIC_SPEC 'Legacy DB Source')"

section "3. Providing ETL Pipeline Services"

provide "etl-orchestrator" "main" "$ETL_ORCHESTRATOR_SPEC"
provide "etl-cleaner"      "main" "$(GENERIC_SPEC 'ETL Cleaner')"
provide "etl-validator"    "main" "$(GENERIC_SPEC 'ETL Validator')"
provide "etl-geo"          "main" "$(GENERIC_SPEC 'ETL Geo Enricher')"
provide "etl-user-segment" "main" "$(GENERIC_SPEC 'ETL User Segmenter')"
provide "etl-purger"       "main" "$(GENERIC_SPEC 'Data Purging Service')"
provide "rule-engine"      "main" "$(GENERIC_SPEC 'Business Rules Engine')"

section "4. Providing ML System Services"

provide "ml-feature-store" "main" "$(GENERIC_SPEC 'ML Feature Store')"
provide "ml-training"      "main" "$(GENERIC_SPEC 'ML Model Training')"
provide "ml-inference"     "main" "$ML_INFERENCE_SPEC"
provide "ml-ab-testing"    "main" "$(GENERIC_SPEC 'ML AB Testing Service')"

section "5. Providing Application & Presentation Services"

provide "user-profile-service" "main" "$(GENERIC_SPEC 'User Profile Service')"
provide "api-middleware"       "main" "$(GENERIC_SPEC 'API Middleware')"
# frontend is usually a client, but can also provide a manifest or similar
provide "frontend-ui"          "main" "$(GENERIC_SPEC 'Web Frontend Assets')"

section "6. Providing Export Services"

provide "export-s3"      "main" "$(GENERIC_SPEC 'S3 Export Service')"
provide "export-bi"      "main" "$(GENERIC_SPEC 'BI Tool Export')"
provide "export-partner" "main" "$(GENERIC_SPEC 'Partner API Export')"

section "7. Providing Observability"
# These are mostly clients but we provide them to have them in the system
provide "monitoring-service" "main" "$(GENERIC_SPEC 'Monitoring & Metrics')"
provide "logging-aggregator" "main" "$(GENERIC_SPEC 'Log Aggregator')"

# Total services provided: 
# 1-8 (Infra/Core) + 9-11 (Ingestion) + 12-18 (ETL) + 19-22 (ML) + 23-25 (App) + 26-28 (Export) + 29-30 (Obs) = 30 services.

section "8. Registering Complex Dependencies"

echo ">>> Setting up Frontend -> Middleware -> Backend chain"
require_endpoint "frontend-ui"    "api-middleware" "main" "/api/v1/resource" "GET"
require_endpoint "api-middleware" "auth-service"   "main" "/api/v1/resource" "POST"
require_endpoint "api-middleware" "user-profile-service" "main" "/api/v1/resource" "GET"
require_endpoint "api-middleware" "elastic-wrapper" "main" "/search" "GET"
require_endpoint "api-middleware" "ml-inference"   "main" "/predict" "POST"

echo ">>> Setting up ETL Pipeline Dependencies"
require_endpoint "etl-orchestrator" "ingestion-stream-a" "main" "/api/v1/resource" "GET"
require_endpoint "etl-orchestrator" "ingestion-stream-b" "main" "/api/v1/resource" "GET"
require_endpoint "etl-orchestrator" "ingestion-legacy"   "main" "/api/v1/resource" "GET"
require_endpoint "etl-orchestrator" "etl-cleaner"        "main" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-validator"      "main" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-geo"            "main" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-user-segment"   "main" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-purger"         "main" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "postgres-wrapper"   "main" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "kafka-wrapper"      "main" "/api/v1/resource" "POST"

require_endpoint "etl-cleaner"      "rule-engine" "main" "/api/v1/resource" "GET"
require_endpoint "etl-geo"          "redis-wrapper" "main" "/api/v1/resource" "GET"
require_endpoint "etl-user-segment" "ml-inference" "main" "/predict" "POST"

echo ">>> Setting up ML System Dependencies"
require_endpoint "ml-inference"     "ml-feature-store" "main" "/api/v1/resource" "GET"
require_endpoint "ml-feature-store" "postgres-wrapper" "main" "/api/v1/resource" "GET"
require_endpoint "ml-training"      "postgres-wrapper" "main" "/api/v1/resource" "GET"
require_endpoint "ml-training"      "ml-feature-store" "main" "/api/v1/resource" "POST"
require_endpoint "ml-ab-testing"    "ml-inference"     "main" "/predict" "POST"

echo ">>> Setting up Exports & Wrappers"
require_endpoint "export-s3"      "postgres-wrapper" "main" "/api/v1/resource" "GET"
require_endpoint "export-bi"      "postgres-wrapper" "main" "/api/v1/resource" "GET"
require_endpoint "export-partner" "kafka-wrapper"    "main" "/api/v1/resource" "GET"
require_endpoint "elastic-wrapper" "postgres-wrapper" "main" "/api/v1/resource" "GET"

echo ">>> Cross-cutting dependencies"
require_endpoint "postgres-wrapper" "audit-service" "main" "/api/v1/resource" "POST"
require_endpoint "api-middleware"   "config-service" "main" "/api/v1/resource" "GET"
require_endpoint "api-middleware"   "secret-manager" "main" "/api/v1/resource" "GET"

echo ">>> Observability"
require_endpoint "monitoring-service" "api-middleware" "main" "/health" "GET"
require_endpoint "monitoring-service" "etl-orchestrator" "main" "/health" "GET"
require_endpoint "logging-aggregator" "api-middleware" "main" "/api/v1/resource" "GET"

echo ""
echo "==========================================================================="
echo "  Demo 2 Scenario complete!"
echo "  Registered 30 services with complex inter-dependencies."
echo "  Open $BASE_URL to see the dependency graph."
echo "==========================================================================="
