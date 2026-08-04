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
#
# 2.0 model: every Provide declares a stability (ga/snapshot) and the version
# is read from the spec's info.version; every require pins an exact version.
# The final section adds a snapshot pin and an Outdated pin for the graph.
# ============================================================================

BASE_URL="${SANSHAIN_URL:-http://localhost:3000}"
ADMIN_USER="${SANSHAIN_USER:-root}"
ADMIN_PASSWORD="${SANSHAIN_PASSWORD:-${INITIAL_ADMIN_PASSWORD:-}}"
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

provide "config-service"   "ga" "$(GENERIC_SPEC 'Config Service')"
provide "secret-manager"   "ga" "$(GENERIC_SPEC 'Secret Manager')"
provide "auth-service"     "ga" "$(GENERIC_SPEC 'Auth Service')"
provide "audit-service"    "ga" "$(GENERIC_SPEC 'Audit Log Service')"
provide "postgres-wrapper" "ga" "$(GENERIC_SPEC 'Postgres DB Wrapper')"
provide "redis-wrapper"    "ga" "$(GENERIC_SPEC 'Redis Cache Wrapper')"
provide "kafka-wrapper"    "ga" "$(GENERIC_SPEC 'Kafka Message Bus')"
provide "elastic-wrapper"  "ga" "$SEARCH_SPEC"

section "2. Providing Data Sources & Ingestion"

provide "ingestion-stream-a" "ga" "$(GENERIC_SPEC 'Data Source A')"
provide "ingestion-stream-b" "ga" "$(GENERIC_SPEC 'Data Source B')"
provide "ingestion-legacy"   "ga" "$(GENERIC_SPEC 'Legacy DB Source')"

section "3. Providing ETL Pipeline Services"

provide "etl-orchestrator" "ga" "$ETL_ORCHESTRATOR_SPEC"
provide "etl-cleaner"      "ga" "$(GENERIC_SPEC 'ETL Cleaner')"
provide "etl-validator"    "ga" "$(GENERIC_SPEC 'ETL Validator')"
provide "etl-geo"          "ga" "$(GENERIC_SPEC 'ETL Geo Enricher')"
provide "etl-user-segment" "ga" "$(GENERIC_SPEC 'ETL User Segmenter')"
provide "etl-purger"       "ga" "$(GENERIC_SPEC 'Data Purging Service')"
provide "rule-engine"      "ga" "$(GENERIC_SPEC 'Business Rules Engine')"

section "4. Providing ML System Services"

provide "ml-feature-store" "ga" "$(GENERIC_SPEC 'ML Feature Store')"
provide "ml-training"      "ga" "$(GENERIC_SPEC 'ML Model Training')"
provide "ml-inference"     "ga" "$ML_INFERENCE_SPEC"
provide "ml-ab-testing"    "ga" "$(GENERIC_SPEC 'ML AB Testing Service')"

section "5. Providing Application & Presentation Services"

provide "user-profile-service" "ga" "$(GENERIC_SPEC 'User Profile Service')"
provide "api-middleware"       "ga" "$(GENERIC_SPEC 'API Middleware')"
# frontend is usually a client, but can also provide a manifest or similar
provide "frontend-ui"          "ga" "$(GENERIC_SPEC 'Web Frontend Assets')"

section "6. Providing Export Services"

provide "export-s3"      "ga" "$(GENERIC_SPEC 'S3 Export Service')"
provide "export-bi"      "ga" "$(GENERIC_SPEC 'BI Tool Export')"
provide "export-partner" "ga" "$(GENERIC_SPEC 'Partner API Export')"

section "7. Providing Observability"
# These are mostly clients but we provide them to have them in the system
provide "monitoring-service" "ga" "$(GENERIC_SPEC 'Monitoring & Metrics')"
provide "logging-aggregator" "ga" "$(GENERIC_SPEC 'Log Aggregator')"

# Total services provided: 
# 1-8 (Infra/Core) + 9-11 (Ingestion) + 12-18 (ETL) + 19-22 (ML) + 23-25 (App) + 26-28 (Export) + 29-30 (Obs) = 30 services.

section "8. Registering Complex Dependencies"

echo ">>> Setting up Frontend -> Middleware -> Backend chain"
require_endpoint "frontend-ui"    "api-middleware" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "api-middleware" "auth-service"   "1.0.0" "/api/v1/resource" "POST"
require_endpoint "api-middleware" "user-profile-service" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "api-middleware" "elastic-wrapper" "1.0.0" "/search" "GET"
require_endpoint "api-middleware" "ml-inference"   "1.0.0" "/predict" "POST"

echo ">>> Setting up ETL Pipeline Dependencies"
require_endpoint "etl-orchestrator" "ingestion-stream-a" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "etl-orchestrator" "ingestion-stream-b" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "etl-orchestrator" "ingestion-legacy"   "1.0.0" "/api/v1/resource" "GET"
require_endpoint "etl-orchestrator" "etl-cleaner"        "1.0.0" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-validator"      "1.0.0" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-geo"            "1.0.0" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-user-segment"   "1.0.0" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "etl-purger"         "1.0.0" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "postgres-wrapper"   "1.0.0" "/api/v1/resource" "POST"
require_endpoint "etl-orchestrator" "kafka-wrapper"      "1.0.0" "/api/v1/resource" "POST"

require_endpoint "etl-cleaner"      "rule-engine" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "etl-geo"          "redis-wrapper" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "etl-user-segment" "ml-inference" "1.0.0" "/predict" "POST"

echo ">>> Setting up ML System Dependencies"
require_endpoint "ml-inference"     "ml-feature-store" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "ml-feature-store" "postgres-wrapper" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "ml-training"      "postgres-wrapper" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "ml-training"      "ml-feature-store" "1.0.0" "/api/v1/resource" "POST"
require_endpoint "ml-ab-testing"    "ml-inference"     "1.0.0" "/predict" "POST"

echo ">>> Setting up Exports & Wrappers"
require_endpoint "export-s3"      "postgres-wrapper" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "export-bi"      "postgres-wrapper" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "export-partner" "kafka-wrapper"    "1.0.0" "/api/v1/resource" "GET"
require_endpoint "elastic-wrapper" "postgres-wrapper" "1.0.0" "/api/v1/resource" "GET"

echo ">>> Cross-cutting dependencies"
require_endpoint "postgres-wrapper" "audit-service" "1.0.0" "/api/v1/resource" "POST"
require_endpoint "api-middleware"   "config-service" "1.0.0" "/api/v1/resource" "GET"
require_endpoint "api-middleware"   "secret-manager" "1.0.0" "/api/v1/resource" "GET"

echo ">>> Observability"
require_endpoint "monitoring-service" "api-middleware" "1.0.0" "/health" "GET"
require_endpoint "monitoring-service" "etl-orchestrator" "1.0.0" "/health" "GET"
require_endpoint "logging-aggregator" "api-middleware" "1.0.0" "/api/v1/resource" "GET"

section "9. Version-model highlights (Snapshot-pinned + Outdated)"

# ml-inference publishes a 1.1.0 snapshot with a new batch endpoint;
# ml-ab-testing opts in -> Snapshot-pinned in the graph.
ML_INFERENCE_SPEC_V11='openapi: 3.0.0
info:
  title: ML Inference API
  version: 1.1.0
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
          description: Prediction result
  /predict/batch:
    post:
      summary: Run batch prediction
      responses:
        "202":
          description: Accepted'

provide "ml-inference" "snapshot" "$ML_INFERENCE_SPEC_V11"
require_endpoint "ml-ab-testing" "ml-inference" "1.1.0" "/predict/batch" "POST"

# etl-orchestrator releases 1.1.0 as GA; monitoring-service stays pinned to
# 1.0.0 -> Outdated in the graph (a display state, still served unchanged).
ETL_ORCHESTRATOR_SPEC_V11="${ETL_ORCHESTRATOR_SPEC/version: 1.0.0/version: 1.1.0}
  /jobs/{id}/retry:
    post:
      summary: Retry a failed job
      responses:
        \"202\":
          description: Retry scheduled"

provide "etl-orchestrator" "ga" "$ETL_ORCHESTRATOR_SPEC_V11"
echo "    monitoring-service still pins etl-orchestrator@1.0.0 -> Outdated highlight."

echo ""
echo "==========================================================================="
echo "  Demo 2 Scenario complete!"
echo "  Registered 30 services with complex inter-dependencies,"
echo "  one Snapshot-pinned dependency and one Outdated pin."
echo "  Open $BASE_URL to see the dependency graph."
echo "==========================================================================="
