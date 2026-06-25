#!/bin/bash
set -e

# Configuration
export INITIAL_ADMIN_PASSWORD=${INITIAL_ADMIN_PASSWORD:-root_password}
export DATABASE_URL="sqlite:sanshain.db?mode=rwc"
PORT=3000

echo "Starting Sanshain Service Release Screenshot Capture..."

# Cleanup old database
rm -f sanshain.db

# Start service in background
# Pre-build to avoid race conditions and slow starts
cargo build
target/debug/sanshain_service &
SVC_PID=$!

# Ensure cleanup on exit
trap "kill $SVC_PID || true" EXIT

# Wait for service to be healthy
echo "Waiting for service to start on port $PORT..."
MAX_RETRIES=30
RETRY_COUNT=0
until curl -s http://localhost:$PORT/health > /dev/null; do
  sleep 2
  RETRY_COUNT=$((RETRY_COUNT + 1))
  if [ $RETRY_COUNT -ge $MAX_RETRIES ]; then
    echo "Service failed to start."
    exit 1
  fi
done

echo "Service started. Logging in to get admin token..."

# Login to get token
LOGIN_RESPONSE=$(curl -s -X POST http://localhost:$PORT/auth/login \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"root\",\"password\":\"$INITIAL_ADMIN_PASSWORD\"}")

TOKEN=$(echo $LOGIN_RESPONSE | jq -r .token)

if [ "$TOKEN" == "null" ] || [ -z "$TOKEN" ]; then
  echo "Login failed. Response: $LOGIN_RESPONSE"
  exit 1
fi

export SANSHAIN_TOKEN=$TOKEN

echo "Enabling Dev Mode for demo scripts..."
curl -s -X POST http://localhost:$PORT/admin/settings/dev-mode \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"enabled":true}'
sleep 2

echo "Populating rich data via demo scripts..."
# Unset token to force demo scripts to login themselves using the password
unset SANSHAIN_TOKEN
export SANSHAIN_USER=root
export SANSHAIN_PASSWORD=$INITIAL_ADMIN_PASSWORD
# Skip basic demo, go straight to complex ones
./scripts/demo2.sh
./scripts/demo_protocols.sh

echo "Applying service metadata for better graph visualization..."
# config-service -> Infrastructure
curl -s -X POST http://localhost:$PORT/admin/services/metadata \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"config-service", "icon":"⚙️", "domain":"Infrastructure"}'

# auth-service -> Core
curl -s -X POST http://localhost:$PORT/admin/services/metadata \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"auth-service", "icon":"🔑", "domain":"Core"}'

# ml-inference -> AI
curl -s -X POST http://localhost:$PORT/admin/services/metadata \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"ml-inference", "icon":"🧠", "domain":"AI"}'

# etl-orchestrator -> Data
curl -s -X POST http://localhost:$PORT/admin/services/metadata \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"etl-orchestrator", "icon":"🏗️", "domain":"Data"}'

echo "Running Playwright screenshot tests..."
# Ensure screenshots directory exists
mkdir -p docs/images

# Run the screenshot test
# We pass the token via env var
INITIAL_ADMIN_PASSWORD=$INITIAL_ADMIN_PASSWORD SANSHAIN_TOKEN=$TOKEN npx playwright test tests/ui/screenshots.test.js --project=chromium

echo "Screenshots updated successfully in docs/images/"
