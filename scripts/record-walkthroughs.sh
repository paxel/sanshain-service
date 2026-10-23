#!/usr/bin/env bash
# Records the two documentation walkthroughs (issue #57): the Developer story
# (site/developer/story.html) and the Operator story (site/operator/story.html).
#
# Builds the release binary, starts a fresh instance on a free port with a
# temporary SQLite file and a known admin password, drives both stories step by
# step and writes what the service really answered into site/evidence/<story>/:
#
#   <nn>-<step>.http   one HTTP exchange: request line, the relevant headers,
#                      the pretty-printed body; then the status line, the
#                      relevant response headers, the body
#   <nn>-<step>.txt    a command transcript (a client invocation, helm template,
#                      the restore procedure, a verification)
#   <nn>-<step>.png    a screenshot, taken by tests/ui/walkthroughs.test.js
#                      (light theme, 1280 wide; `-dark.png` where a twin matters)
#
# Every recorded exchange declares the status and a few body fields it expects;
# a mismatch stops the script with the step name and the difference, so a
# behaviour change is noticed instead of silently recorded. The run is
# idempotent: the evidence folders are emptied first, the temporary instance is
# killed on exit, nothing else in the repository is touched except the
# chapter screenshots the Playwright spec writes into site/assets/images/.
#
# Toolchains (the recording sandbox has them without root; the script falls
# back to curl, and says so in the transcript, when a client is missing):
#   cargo            release build of the service; `cargo sanshain` from
#                    CARGO_SANSHAIN (default: `cargo sanshain` on PATH)
#   node, npx        the JS client (SANSHAIN_JS, default: `sanshain` on PATH)
#                    and Playwright (tests/ui/walkthroughs.test.js, chromium)
#   go               only to build the Go client when SANSHAIN_GO is unset and
#                    `sanshain-go` is not on PATH (go install …/cmd/sanshain-go)
#   java + mvn       the Maven plugin (MAVEN_PLUGIN, default
#                    io.github.paxel.sanshain:sanshain-maven-plugin:2.2.0),
#                    run offline when the plugin is in ~/.m2 (MVN_FLAGS)
#   python           the Conan client (SANSHAIN_CONAN, default: `sanshain-conan`
#                    on PATH) and the signature check
#   helm             `helm template` for the install transcript (HELM)
#   curl, jq         every HTTP step and the expectations
#
# Environment: SANSHAIN_PORT (default: a free port), RECORD_SKIP_BUILD=1 to use
# the existing target/release binary, CARGO_BUILD_CMD to override the build
# command (e.g. a niced one), PLAYWRIGHT_ARGS for extra Playwright arguments.
#
# Usage: ./scripts/record-walkthroughs.sh   (also: just record-walkthroughs,
#        npm run record-walkthroughs)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

EVIDENCE="$REPO_ROOT/site/evidence"
DEV="$EVIDENCE/developer"
OPS="$EVIDENCE/operator"
ADMIN_PASSWORD="walkthrough-admin"
NEW_ADMIN_PASSWORD="walkthrough-admin-2026"
MARIA_PASSWORD="maria-walkthrough"
PUBLIC_BASE_URL="https://sanshain.corp"
BRANCH="release-2026.10"
MAVEN_PLUGIN="${MAVEN_PLUGIN:-io.github.paxel.sanshain:sanshain-maven-plugin:2.2.0}"
HELM="${HELM:-helm}"

# ---------------------------------------------------------------------------
# Tools

need() { command -v "$1" >/dev/null 2>&1 || { echo "record-walkthroughs: $1 is required" >&2; exit 1; }; }
need curl; need jq; need node; need python3

free_port() {
  node -e 'const s=require("net").createServer();s.listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})'
}

TMP="$(mktemp -d -t sanshain-walkthrough-XXXXXX)"
PORT="${SANSHAIN_PORT:-$(free_port)}"
# The receiver's port is fixed so the hook URL in the screenshots is stable
# across recordings; the text evidence names it receiver.sanshain.svc.
RECEIVER_PORT="${RECEIVER_PORT:-18080}"
node -e 'const s=require("net").createServer();s.once("error",()=>process.exit(1));s.listen(Number(process.argv[1]),"127.0.0.1",()=>{s.close()})' "$RECEIVER_PORT" \
  || fail "port $RECEIVER_PORT (RECEIVER_PORT) is in use"
BASE="http://127.0.0.1:$PORT"
SVC_PID=""
RECEIVER_PID=""
DATA_LINK=""

cleanup() {
  local code=$?
  [ -n "$SVC_PID" ] && kill "$SVC_PID" 2>/dev/null || true
  [ -n "$RECEIVER_PID" ] && kill "$RECEIVER_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  [ -n "$DATA_LINK" ] && rm -f "$DATA_LINK"
  if [ $code -ne 0 ]; then
    echo "record-walkthroughs: FAILED (exit $code); the instance's log is $TMP/service.log" >&2
  else
    rm -rf "$TMP"
  fi
  exit $code
}
trap cleanup EXIT

fail() { echo "record-walkthroughs: $*" >&2; exit 1; }
say() { echo ">>> $*"; }

# ---------------------------------------------------------------------------
# Build and start

if [ "${RECORD_SKIP_BUILD:-}" != "1" ]; then
  say "Building the release binary"
  ${CARGO_BUILD_CMD:-cargo build --release}
fi
[ -x target/release/sanshain_service ] || fail "target/release/sanshain_service is missing"

mkdir -p "$DEV" "$OPS"
find "$DEV" "$OPS" -type f \( -name '*.http' -o -name '*.txt' -o -name '*.png' \) -delete

say "Starting a fresh instance on port $PORT (SQLite in $TMP/data)"
mkdir -p "$TMP/data"
# The service resolves its migrations from the working directory, so it runs
# from the repository root. The database still lives in the temp folder: a
# `data` link (the image's /data), removed on exit, keeps the admin page
# showing `sqlite:data/sanshain.db` instead of this machine's temp path.
DATABASE_URL="sqlite:$TMP/data/sanshain.db?mode=rwc"
[ -L "$REPO_ROOT/data" ] && rm -f "$REPO_ROOT/data"   # a stale link from an aborted run
if [ ! -e "$REPO_ROOT/data" ]; then
  ln -s "$TMP/data" "$REPO_ROOT/data"
  DATA_LINK="$REPO_ROOT/data"
  DATABASE_URL="sqlite:data/sanshain.db?mode=rwc"
fi
(
  export DATABASE_URL
  export BIND_ADDRESS="127.0.0.1:$PORT"
  export INITIAL_ADMIN_PASSWORD="$ADMIN_PASSWORD"
  export INITIAL_AUTH_MODE=local
  export PUBLIC_BASE_URL
  export WEBHOOK_WORKER_INTERVAL_SECS=1
  export OTEL_ENABLED=false
  export RUST_LOG="info,sanshain_service=info"
  exec nice -n 19 "$REPO_ROOT/target/release/sanshain_service" >"$TMP/service.log" 2>&1
) &
SVC_PID=$!

for _ in $(seq 1 60); do
  if [ "$(curl -s "$BASE/health" 2>/dev/null)" = "OK" ]; then break; fi
  kill -0 "$SVC_PID" 2>/dev/null || fail "the service exited; see $TMP/service.log"
  sleep 1
done
[ "$(curl -s "$BASE/health")" = "OK" ] || fail "the service did not become healthy on $BASE"

# A receiver for the webhook step: records every POST it gets.
mkdir -p "$TMP/deliveries"
cat >"$TMP/receiver.js" <<'EOF'
const http = require("node:http");
const fs = require("node:fs");
const dir = process.argv[3];
let n = 0;
http
  .createServer((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      n += 1;
      fs.writeFileSync(
        `${dir}/${n}.json`,
        JSON.stringify({ method: req.method, url: req.url, headers: req.headers, body }),
      );
      res.writeHead(200, { "content-type": "text/plain" });
      res.end("ok");
    });
  })
  .listen(Number(process.argv[2]), "127.0.0.1");
EOF
node "$TMP/receiver.js" "$RECEIVER_PORT" "$TMP/deliveries" &
RECEIVER_PID=$!

# ---------------------------------------------------------------------------
# Recording helpers

LAST_STATUS=""
LAST_BODY="$TMP/last.body"
LAST_HEADERS="$TMP/last.headers"
REDACT=()   # pairs: secret value -> replacement, applied to every evidence file

redact() { REDACT+=("$1" "$2"); }
# The temporary paths and the random port are this run's, not the reader's:
# file names are shown relative to the project, the URL as the public one.
redact "$TMP/payments/" ""
redact "$TMP/orders/" ""
redact "$TMP/" ""
redact "$BASE" "$PUBLIC_BASE_URL"
# The local receiver stands in for an in-cluster one; the screenshots show the
# real address, the text evidence the name an install would use.
redact "127.0.0.1:$RECEIVER_PORT" "receiver.sanshain.svc"
redact "http://127.0.0.1" "http://receiver.sanshain.svc"
redact "host 127.0.0.1" "host receiver.sanshain.svc"

scrub() {
  local i
  local text
  text="$(cat)"
  for ((i = 0; i < ${#REDACT[@]}; i += 2)); do
    text="${text//"${REDACT[i]}"/${REDACT[i + 1]}}"
  done
  # Credentials by shape, whichever call answered them.
  printf '%s\n' "$text" | sed -E 's/san_[0-9a-f]{64}/san_…/g; s/"token": "[0-9a-f]{64}"/"token": "<session token>"/g'
}

pretty() {
  # JSON is pretty-printed; anything else is passed through.
  if jq -e . >/dev/null 2>&1 <"$1"; then jq . <"$1"; else cat "$1"; echo; fi
}

# request <story-dir> <file> <METHOD> <path> [auth] [body-file] [extra curl args…]
#   auth: "" (none), "token" ($TOKEN), "session" ($SESSION), "maria" ($MARIA_SESSION),
#         "readonly" ($RO_TOKEN); the Authorization value in the evidence is
#         replaced by a placeholder of its kind.
request() {
  local dir="$1" file="$2" method="$3" path="$4" auth="${5:-}" body="${6:-}"
  shift 6 2>/dev/null || shift $#
  local out="$dir/$file.http"
  local -a args=(-sS -o "$LAST_BODY" -D "$LAST_HEADERS" -w '%{http_code}' -X "$method")
  local shown_auth=""
  case "$auth" in
    token) args+=(-H "Authorization: Bearer $TOKEN"); shown_auth="Authorization: Bearer san_…" ;;
    readonly) args+=(-H "Authorization: Bearer $RO_TOKEN"); shown_auth="Authorization: Bearer san_… (read_only)" ;;
    session) args+=(-H "Authorization: Bearer $SESSION"); shown_auth="Authorization: Bearer <session token>" ;;
    maria) args+=(-H "Authorization: Bearer $MARIA_SESSION"); shown_auth="Authorization: Bearer <maria's session token>" ;;
    "") ;;
    *) fail "unknown auth kind $auth" ;;
  esac
  local content_type=""
  if [ -n "$body" ]; then
    content_type="application/json"
    args+=(-H "Content-Type: application/json" --data-binary "@$body")
  fi
  args+=("$@")
  LAST_STATUS="$(curl "${args[@]}" "$BASE$path")" || fail "$dir/$file: curl failed"
  {
    echo "$method $path HTTP/1.1"
    [ -n "$shown_auth" ] && echo "$shown_auth"
    [ -n "$content_type" ] && echo "Content-Type: $content_type"
    local a
    for a in "$@"; do
      case "$a" in Accept:*|If-None-Match:*|Host:*) echo "$a" ;; esac
    done
    if [ -n "$body" ]; then
      echo
      pretty "$body"
    fi
    echo
    head -n 1 "$LAST_HEADERS" | tr -d '\r'
    grep -i -E '^(content-type|content-disposition|content-length|etag|location|www-authenticate|x-sanshain-[a-z-]+|sanshain-warning):' "$LAST_HEADERS" \
      | tr -d '\r' | sed -E 's/^([A-Za-z-]+):/\1:/' || true
    echo
    if grep -qi '^content-type: application/vnd.sqlite3' "$LAST_HEADERS"; then
      echo "<$(stat -c %s "$LAST_BODY") bytes of SQLite database, saved as sanshain-backup.db>"
    else
      pretty "$LAST_BODY"
    fi
  } | scrub >"$out"
  echo "    $method $path -> $LAST_STATUS  ($(basename "$out"))"
}

# expect <step> <status> [jq expression…]: every expression must be true for the last body.
expect() {
  local step="$1" status="$2"
  shift 2
  if [ "$LAST_STATUS" != "$status" ]; then
    echo "record-walkthroughs: step $step: expected HTTP $status, got $LAST_STATUS" >&2
    echo "--- body:" >&2
    cat "$LAST_BODY" >&2
    echo >&2
    exit 1
  fi
  local e
  for e in "$@"; do
    if ! jq -e "$e" "$LAST_BODY" >/dev/null 2>&1; then
      echo "record-walkthroughs: step $step: the body does not satisfy: $e" >&2
      echo "--- body:" >&2
      jq . "$LAST_BODY" 2>/dev/null >&2 || cat "$LAST_BODY" >&2
      exit 1
    fi
  done
}

# expect_empty <step>: the last answer had no body.
expect_empty() {
  if [ -s "$LAST_BODY" ]; then
    echo "record-walkthroughs: step $1: expected an empty body, got:" >&2
    cat "$LAST_BODY" >&2
    exit 1
  fi
}

# expect_text <step> <pattern>: the last (non-JSON) body contains the line.
expect_text() {
  grep -qF -- "$2" "$LAST_BODY" || {
    echo "record-walkthroughs: step $1: the body does not contain: $2" >&2
    cat "$LAST_BODY" >&2
    exit 1
  }
}

# shots <playwright args…>: the UI steps, against the running instance.
shots() {
  local session
  session="$(login root "$ADMIN_PASSWORD")"
  # shellcheck disable=SC2086
  BASE_URL="$BASE" SANSHAIN_TOKEN="$session" EVIDENCE_DIR="$EVIDENCE" \
    npx playwright test tests/ui/walkthroughs.test.js --project=chromium --workers=1 ${PLAYWRIGHT_ARGS:-} "$@"
}

# transcript <story-dir> <file> <label> <command…>: runs the command in $WORKDIR
# and records `$ command` with its output; a failing command fails the step.
# SHOW, when set, is the command line as the reader would type it (the real
# one may name a binary by its path).
transcript() {
  local dir="$1" file="$2" label="$3"
  shift 3
  local out="$dir/$file.txt"
  local log="$TMP/transcript.log"
  {
    [ -n "$label" ] && echo "# $label"
    echo "\$ ${SHOW:-$*}"
  } >"$log"
  if ! (cd "${WORKDIR:-$TMP}" && "$@") >>"$log" 2>&1; then
    echo "record-walkthroughs: step $dir/$file: the command failed:" >&2
    cat "$log" >&2
    exit 1
  fi
  scrub <"$log" >"$out"
  echo "    $* ($(basename "$out"))"
}

# transcript_curl_fallback <story-dir> <file> <client> <invocation>: used when a
# client is not available; the HTTP exchange is recorded separately.
note_fallback() {
  local dir="$1" file="$2" client="$3" invocation="$4"
  printf '# recorded with curl; the %s invocation is:\n$ %s\n' "$client" "$invocation" >"$dir/$file.txt"
  echo "    ($client not available: $file.txt notes the invocation)"
}

login() {
  curl -sS -X POST "$BASE/auth/login" -H "Content-Type: application/json" \
    -d "{\"username\":\"$1\",\"password\":\"$2\"}" | jq -r .token
}

# ---------------------------------------------------------------------------
# Clients

CARGO_SANSHAIN="${CARGO_SANSHAIN:-}"
if [ -z "$CARGO_SANSHAIN" ] && cargo sanshain --help >/dev/null 2>&1; then CARGO_SANSHAIN="cargo sanshain"; fi
SANSHAIN_JS="${SANSHAIN_JS:-}"
if [ -z "$SANSHAIN_JS" ] && command -v sanshain >/dev/null 2>&1; then SANSHAIN_JS="sanshain"; fi
SANSHAIN_GO="${SANSHAIN_GO:-}"
if [ -z "$SANSHAIN_GO" ] && command -v sanshain-go >/dev/null 2>&1; then SANSHAIN_GO="sanshain-go"; fi
SANSHAIN_CONAN="${SANSHAIN_CONAN:-}"
if [ -z "$SANSHAIN_CONAN" ] && command -v sanshain-conan >/dev/null 2>&1; then SANSHAIN_CONAN="sanshain-conan"; fi
MVN="${MVN:-}"
if [ -z "$MVN" ] && command -v mvn >/dev/null 2>&1; then MVN="mvn"; fi

# ---------------------------------------------------------------------------
# The team's files

mkdir -p "$TMP/payments" "$TMP/orders"
spec_payments() {
  # spec_payments <version> <variant>: v1 (two endpoints), v1-plus (a third),
  # v2 (the third, without GET /payments/{id}), v2-draft (adds a refund)
  local version="$1" variant="$2"
  {
    cat <<EOF
openapi: 3.0.3
info:
  title: Payments
  version: $version
paths:
  /payments:
    post:
      summary: Create a payment
      responses:
        "201":
          description: Created
EOF
    if [ "$variant" != "v2" ] && [ "$variant" != "v2-draft" ]; then
      cat <<'EOF'
  /payments/{id}:
    get:
      summary: One payment
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
      responses:
        "200":
          description: OK
EOF
    else
      cat <<'EOF'
  /payments/{paymentId}:
    get:
      summary: One payment
      parameters:
        - name: paymentId
          in: path
          required: true
          schema: { type: string }
      responses:
        "200":
          description: OK
EOF
    fi
    if [ "$variant" != "v1" ]; then
      cat <<'EOF'
  /payments/{id}/capture:
    post:
      summary: Capture an authorised payment
      responses:
        "200":
          description: OK
EOF
    fi
    if [ "$variant" = "v2-draft" ]; then
      cat <<'EOF'
  /payments/{id}/refund:
    post:
      summary: Refund a captured payment
      responses:
        "200":
          description: OK
EOF
    fi
  }
}

provide_body() {
  # provide_body <spec-file> <stability> [trunk]
  if [ "${3:-}" = "trunk" ]; then
    jq -n --arg y "$(cat "$1")" --arg st "$2" '{producername:"payments", stability:$st, openapi_yaml:$y, trunk:true}'
  else
    jq -n --arg y "$(cat "$1")" --arg st "$2" '{producername:"payments", stability:$st, openapi_yaml:$y}'
  fi
}

cat >"$TMP/payments/sanshain.yaml" <<'EOF'
serviceName: payments
catalogInfo: catalog-info.yaml
provides:
  - file: openapi.yaml        # version read from info.version
EOF
cat >"$TMP/payments/catalog-info.yaml" <<'EOF'
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: payments
  description: Takes payments for the shop and hands them to the bank.
  annotations:
    backstage.io/source-location: url:https://github.com/acme/payments/tree/main/
  links:
    - url: https://wiki.acme.corp/payments
      title: Runbook
      type: docs
    - url: https://grafana.acme.corp/d/payments
      title: Dashboard
spec:
  type: service
  owner: team-payments
  dependsOn:
    - resource:payments-db
    - resource:bank-gateway
EOF
cat >"$TMP/orders/sanshain.yaml" <<'EOF'
serviceName: orders
requires:
  - serviceName: payments
    apiType: openapi
    version: 1.0.0           # the pin
    outputDirectory: generated/payments
    endpoints:
      - method: POST
        path: /payments
EOF

export SANSHAIN_URL="$BASE"

# ===========================================================================
# Developer story
# ===========================================================================
say "Developer story"

SESSION="$(login root "$ADMIN_PASSWORD")"
[ -n "$SESSION" ] && [ "$SESSION" != "null" ] || fail "root login failed"
redact "$SESSION" "<session token>"
TOKEN="$(curl -sS -X POST "$BASE/auth/tokens" -H "Authorization: Bearer $SESSION" -H "Content-Type: application/json" \
  -d '{"name":"payments-ci","expires_in_days":30}' | jq -r .token)"
[ "${TOKEN:0:4}" = "san_" ] || fail "minting the CI token failed"
redact "$TOKEN" "san_…"
export SANSHAIN_TOKEN="$TOKEN"

# 01 — the first provide, as a snapshot, with cargo-sanshain
spec_payments 1.0.0 v1 >"$TMP/payments/openapi.yaml"
cp "$TMP/payments/sanshain.yaml" "$DEV/01-sanshain-yaml.txt"
cp "$TMP/payments/openapi.yaml" "$DEV/01-openapi-yaml.txt"
if [ -n "$CARGO_SANSHAIN" ]; then
  # shellcheck disable=SC2086
  SHOW="cargo sanshain provide --trunk" WORKDIR="$TMP/payments" transcript "$DEV" "01-first-provide" "payments/, trunk CI of the payments repository" $CARGO_SANSHAIN provide --trunk
else
  note_fallback "$DEV" "01-first-provide" "cargo-sanshain" "cargo sanshain provide --trunk"
  provide_body "$TMP/payments/openapi.yaml" snapshot trunk >"$TMP/body.json"
  request "$DEV" "01-first-provide-curl" POST /provide token "$TMP/body.json"
  expect "developer/01" 202 '.version == "1.0.0"' '.stability == "snapshot"'
fi
request "$DEV" "01-versions" GET "/producers/payments/versions" token ""
expect "developer/01-versions" 200 'map(select(.version == "1.0.0" and .stability == "snapshot")) | length == 1'

# 02 — the same number as GA: promoted in place
note_fallback "$DEV" "02-publish-ga" "cargo-sanshain" "cargo sanshain provide --ga --trunk"
provide_body "$TMP/payments/openapi.yaml" ga trunk >"$TMP/body.json"
request "$DEV" "02-publish-ga" POST /provide token "$TMP/body.json"
expect "developer/02" 202 '.version == "1.0.0"' '.stability == "ga"' '.promoted == true'

# 03 — orders pins POST /payments at 1.0.0 with the JS client, then the same require by hand
if [ -n "$SANSHAIN_JS" ]; then
  # shellcheck disable=SC2086
  SHOW="sanshain --trunk require" WORKDIR="$TMP/orders" transcript "$DEV" "03-orders-requires" "orders/, trunk CI of the orders repository" $SANSHAIN_JS --trunk require
  {
    echo "\$ cat generated/payments/payments.yaml"
    cat "$TMP/orders/generated/payments/"*.yaml 2>/dev/null || fail "the JS client wrote no snippet"
  } >>"$DEV/03-orders-requires.txt"
else
  note_fallback "$DEV" "03-orders-requires" "sanshain-js" "sanshain --trunk require"
fi
request "$DEV" "03-require" GET "/require?consumername=orders&producername=payments&version=1.0.0&path=/payments&method=POST&trunk=true" token ""
expect "developer/03" 200
expect_text "developer/03" "summary: Create a payment"
grep -qi '^x-sanshain-stability: ga' "$LAST_HEADERS" || fail "developer/03: expected x-sanshain-stability: ga"
# The graph screenshot of this step, taken now: the periphery arrives in step 10.
shots -g "developer 03"

# 04 — a forgotten bump: new content under the GA number
note_fallback "$DEV" "04-forgotten-bump" "cargo-sanshain" "cargo sanshain provide --ga --trunk"
spec_payments 1.0.0 v1-plus >"$TMP/draft.yaml"
provide_body "$TMP/draft.yaml" ga trunk >"$TMP/body.json"
request "$DEV" "04-forgotten-bump" POST /provide token "$TMP/body.json"
expect "developer/04" 409 '.proposed_version == "1.1.0"' '.error | test("did you forget")'

# 05 — a breaking change behind a minor bump
note_fallback "$DEV" "05-breaking-minor" "cargo-sanshain" "cargo sanshain provide --ga --trunk"
spec_payments 1.1.0 v2 >"$TMP/draft.yaml"
provide_body "$TMP/draft.yaml" ga trunk >"$TMP/body.json"
request "$DEV" "05-breaking-minor" POST /provide token "$TMP/body.json"
expect "developer/05" 409 '.proposed_version == "2.0.0"'

# 06 — 2.0.0 accepted, published GA by the Go client
spec_payments 2.0.0 v2 >"$TMP/payments/openapi.yaml"
if [ -n "$SANSHAIN_GO" ]; then
  SHOW="SANSHAIN_GA=true sanshain-go -trunk provide" WORKDIR="$TMP/payments" transcript "$DEV" "06-major-ga" "payments/, release pipeline" env SANSHAIN_GA=true "$SANSHAIN_GO" -trunk provide
else
  note_fallback "$DEV" "06-major-ga" "sanshain-go" "SANSHAIN_GA=true sanshain-go -trunk provide"
  provide_body "$TMP/payments/openapi.yaml" ga trunk >"$TMP/body.json"
  request "$DEV" "06-major-ga-curl" POST /provide token "$TMP/body.json"
  expect "developer/06" 202 '.version == "2.0.0"'
fi
request "$DEV" "06-versions" GET "/producers/payments/versions" token ""
expect "developer/06-versions" 200 'map(select(.version == "2.0.0" and .stability == "ga")) | length == 1'

# 07 — a snapshot is overwritten, then promoted
note_fallback "$DEV" "07-snapshot" "cargo-sanshain" "cargo sanshain provide            # twice, as the draft grows; then: cargo sanshain provide --ga --trunk"
spec_payments 2.1.0 v2 >"$TMP/draft.yaml"
provide_body "$TMP/draft.yaml" snapshot >"$TMP/body.json"
request "$DEV" "07-snapshot" POST /provide token "$TMP/body.json"
expect "developer/07-snapshot" 202 '.version == "2.1.0"' '.stability == "snapshot"'
spec_payments 2.1.0 v2-draft >"$TMP/draft.yaml"
provide_body "$TMP/draft.yaml" snapshot >"$TMP/body.json"
request "$DEV" "07-snapshot-overwrite" POST /provide token "$TMP/body.json"
expect "developer/07-overwrite" 202 '.version == "2.1.0"' '.stability == "snapshot"' '.changes.inserts >= 1'
provide_body "$TMP/draft.yaml" ga trunk >"$TMP/body.json"
request "$DEV" "07-promote" POST /provide token "$TMP/body.json"
expect "developer/07-promote" 202 '.version == "2.1.0"' '.stability == "ga"' '.promoted == true'
cp "$TMP/draft.yaml" "$TMP/payments/openapi.yaml"

# 08 — the diff between two versions and one endpoint's history
request "$DEV" "08-diff" GET "/admin/producers/payments/diff?api_type=openapi&from=1.0.0&to=2.0.0" session ""
expect "developer/08-diff" 200
expect_text "developer/08-diff" "+  version: 2.0.0"
expect_text "developer/08-diff" "-  /payments/{id}:"
request "$DEV" "08-endpoint-history" GET "/endpoint-versions?service=payments&api_type=openapi&path=/payments&method=POST" token ""
expect "developer/08-history" 200 'length == 3' '.[0].version == "1.0.0"' 'all(.stability == "ga")'

# 09 — a release branch is cut; a report scoped to it; a hotfix require on it
printf '{"name":"%s"}' "$BRANCH" >"$TMP/body.json"
request "$DEV" "09-branch-cut" POST /admin/branches session "$TMP/body.json"
expect "developer/09-branch" 201 ".name == \"$BRANCH\""
request "$DEV" "09-scoped-report" GET "/report?scope=$BRANCH" token ""
expect "developer/09-report" 200 ".scope_label == \"$BRANCH\"" '.dependency_graph | length == 1' '.dependency_graph[0] | .client == "orders" and .version == "1.0.0"'
if [ -n "$SANSHAIN_CONAN" ]; then
  SHOW="sanshain-conan --tag $BRANCH require" WORKDIR="$TMP/orders" transcript "$DEV" "09-branch-require" "orders/, hotfix pipeline of $BRANCH" "$SANSHAIN_CONAN" --tag "$BRANCH" require
else
  note_fallback "$DEV" "09-branch-require" "sanshain-conan" "sanshain-conan --tag $BRANCH require"
fi

# 10 — the catalog push with the Maven plugin; the context it recorded
if [ -n "$MVN" ]; then
  cat >"$TMP/payments/pom.xml" <<'EOF'
<project xmlns="http://maven.apache.org/POM/4.0.0">
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.acme</groupId>
    <artifactId>payments</artifactId>
    <version>2.1.0</version>
</project>
EOF
  cat >"$TMP/payments/settings.xml" <<EOF
<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0">
    <servers>
        <server>
            <id>sanshain</id>
            <password>\${env.SANSHAIN_TOKEN}</password>
            <configuration>
                <url>$BASE</url>
            </configuration>
        </server>
    </servers>
</settings>
EOF
  # shellcheck disable=SC2086
  SHOW="mvn -s settings.xml -Dsanshain.trunk=true $MAVEN_PLUGIN:catalog" WORKDIR="$TMP/payments" transcript "$DEV" "10-catalog-push" "payments/, trunk CI" "$MVN" ${MVN_FLAGS:-} -s settings.xml -Dsanshain.trunk=true "$MAVEN_PLUGIN:catalog"
else
  note_fallback "$DEV" "10-catalog-push" "the Maven plugin" "mvn $MAVEN_PLUGIN:catalog -Dsanshain.trunk=true"
  jq -n --arg y "$(cat "$TMP/payments/catalog-info.yaml")" '{catalog_info_yaml:$y, trunk:true}' >"$TMP/body.json"
  request "$DEV" "10-catalog-push-curl" PUT /catalog/payments token "$TMP/body.json"
  expect "developer/10-push" 200 '.recorded == true'
fi
request "$DEV" "10-catalog-context" GET /admin/producers/payments session ""
expect "developer/10-context" 200 '.context.owner.name == "team-payments"' '.context.links | length == 2'

# 11 — the free validator on a draft, before publishing
spec_payments 2.2.0 v2-draft >"$TMP/draft.yaml"
jq -n --arg c "$(cat "$TMP/draft.yaml")" '{api_type:"openapi", content:$c}' >"$TMP/body.json"
request "$DEV" "11-validate" POST /validate "" "$TMP/body.json"
expect "developer/11-validate" 200 '.valid == true' '.version == "2.2.0"'
sed '/^  version:/d' "$TMP/draft.yaml" >"$TMP/draft-noversion.yaml"
jq -n --arg c "$(cat "$TMP/draft-noversion.yaml")" '{api_type:"openapi", content:$c}' >"$TMP/body.json"
request "$DEV" "11-validate-invalid" POST /validate "" "$TMP/body.json"
expect "developer/11-invalid" 200 '.valid == false'

# 12 — a read-only token for an AI client; a question over MCP; the token refused on a write
printf '{"name":"claude-code","expires_in_days":90,"scope":"read_only"}' >"$TMP/body.json"
request "$DEV" "12-read-only-token" POST /auth/tokens session "$TMP/body.json"
expect "developer/12-token" 200 '.scope == "read_only"'
RO_TOKEN="$(jq -r .token "$LAST_BODY")"
redact "$RO_TOKEN" "san_…"
cat >"$TMP/body.json" <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"describe_service","arguments":{"name":"payments"}}}
EOF
request "$DEV" "12-mcp-describe" POST /mcp readonly "$TMP/body.json" -H "Accept: application/json, text/event-stream"
expect "developer/12-mcp" 200 '.result.content[0].text | fromjson | .context.owner.name == "team-payments"'
provide_body "$TMP/payments/openapi.yaml" ga >"$TMP/body.json"
request "$DEV" "12-read-only-refused" POST /provide readonly "$TMP/body.json"
expect "developer/12-refused" 403 '.error | test("read_only")'

# ===========================================================================
# Operator story
# ===========================================================================
say "Operator story"

# 01 — the Helm install: a template render, not a cluster
if command -v "$HELM" >/dev/null 2>&1; then
  SHOW="helm template sanshain deploy/helm/sanshain --set secrets.databaseUrl=postgres://sanshain:secret@postgres.sanshain.svc:5432/sanshain --set secrets.initialAdminPassword=walkthrough-admin --set ingress.enabled=true --set 'ingress.hosts[0].host=sanshain.corp' --set 'ingress.hosts[0].paths[0].path=/' --set 'ingress.hosts[0].paths[0].pathType=Prefix' --set backup.enabled=true --set backup.engine=postgres" \
  WORKDIR="$REPO_ROOT" transcript "$OPS" "01-helm-template" "Not recorded against a cluster: this is what the chart renders for these values" \
    "$HELM" template sanshain deploy/helm/sanshain \
    --set secrets.databaseUrl=postgres://sanshain:secret@postgres.sanshain.svc:5432/sanshain \
    --set secrets.initialAdminPassword="$ADMIN_PASSWORD" \
    --set ingress.enabled=true --set 'ingress.hosts[0].host=sanshain.corp' \
    --set 'ingress.hosts[0].paths[0].path=/' --set 'ingress.hosts[0].paths[0].pathType=Prefix' \
    --set backup.enabled=true --set backup.engine=postgres
else
  note_fallback "$OPS" "01-helm-template" "helm" "helm template sanshain deploy/helm/sanshain --set secrets.databaseUrl=postgres://… --set backup.enabled=true"
fi

# 02 — the first login and the admin password
grep -h 'INITIAL SETUP' "$TMP/service.log" | sed -E 's/^.*(\[INITIAL SETUP\])/\1/' | scrub >"$OPS/02-initial-setup.txt"
[ -s "$OPS/02-initial-setup.txt" ] || fail "operator/02: the service log has no [INITIAL SETUP] lines"
printf '{"username":"root","password":"%s"}' "$ADMIN_PASSWORD" >"$TMP/body.json"
request "$OPS" "02-first-login" POST /auth/login "" "$TMP/body.json"
expect "operator/02-login" 200 '.token | length > 0'
printf '{"old_password":"%s","new_password":"%s"}' "$ADMIN_PASSWORD" "$NEW_ADMIN_PASSWORD" >"$TMP/body.json"
request "$OPS" "02-change-password" POST /auth/change-password session "$TMP/body.json"
expect "operator/02-password" 200 '.token | length > 0'
ADMIN_PASSWORD="$NEW_ADMIN_PASSWORD"
# A password change ends every session of the account: sign in again.
request "$OPS" "02-old-session-gone" GET /auth/me session ""
expect "operator/02-session-gone" 401
expect_empty "operator/02-session-gone"
SESSION="$(login root "$ADMIN_PASSWORD")"
[ -n "$SESSION" ] && [ "$SESSION" != "null" ] || fail "root login with the new password failed"
redact "$SESSION" "<session token>"

# 03 — OpenID Connect: configuration only, no identity provider in this recording
cat >"$TMP/body.json" <<EOF
{
  "auth_mode": "oidc",
  "oidc_config": {
    "issuer_url": "https://login.acme.corp/realms/corp",
    "client_id": "sanshain",
    "client_secret": "oidc-client-secret-from-the-provider",
    "redirect_url": "$PUBLIC_BASE_URL/auth/oidc/callback",
    "username_claim": "preferred_username",
    "groups_claim": "groups",
    "admin_group": "sanshain-admins"
  }
}
EOF
request "$OPS" "03-oidc-configure" PUT /admin/auth-config session "$TMP/body.json"
expect "operator/03-configure" 200
expect_empty "operator/03-configure"
request "$OPS" "03-oidc-state" GET /admin/auth-config session ""
expect "operator/03-state" 200 '.auth_mode == "oidc"' '.oidc_config.client_secret == "****"'
# Back to local accounts: in oidc mode nobody can register locally, and the
# rest of the story registers and logs in locally. The stored OIDC values stay.
printf '{"auth_mode":"local"}' >"$TMP/body.json"
request "$OPS" "03-oidc-back-to-local" PUT /admin/auth-config session "$TMP/body.json"
expect "operator/03-back-to-local" 200
curl -sS "$BASE/admin/auth-config" -H "Authorization: Bearer $SESSION" | jq -e '.auth_mode == "local" and .oidc_config.client_id == "sanshain"' >/dev/null \
  || fail "operator/03: expected auth_mode local with the OIDC values kept"

# 04 — a user, a role, a maintainer scope, and what that user may do now
printf '{"enabled":true}' >"$TMP/body.json"
request "$OPS" "04-open-registration" POST /admin/settings/registration session "$TMP/body.json"
expect "operator/04-open-registration" 200
printf '{"username":"maria","password":"%s"}' "$MARIA_PASSWORD" >"$TMP/body.json"
request "$OPS" "04-register" POST /auth/register "" "$TMP/body.json"
expect "operator/04-register" 201 '.approved == false'
request "$OPS" "04-users" GET /admin/users session ""
expect "operator/04-users" 200 'map(select(.username == "maria")) | length == 1'
MARIA_ID="$(jq -r '.[] | select(.username == "maria") | .id' "$LAST_BODY")"
request "$OPS" "04-approve" POST "/admin/users/$MARIA_ID/approve" session ""
expect "operator/04-approve" 200
expect_empty "operator/04-approve"
curl -sS "$BASE/admin/users" -H "Authorization: Bearer $SESSION" | jq -e '.[] | select(.username == "maria") | .approved == true' >/dev/null \
  || fail "operator/04: maria is not approved after the approve call"
printf '{"role":"releaser"}' >"$TMP/body.json"
request "$OPS" "04-grant-role" POST "/admin/users/$MARIA_ID/roles" session "$TMP/body.json"
expect "operator/04-role" 201
expect_empty "operator/04-role"
printf '{"user_id":%s}' "$MARIA_ID" >"$TMP/body.json"
request "$OPS" "04-grant-maintainer" POST /admin/producers/payments/maintainers session "$TMP/body.json"
expect "operator/04-maintainer" 201
expect_empty "operator/04-maintainer"
MARIA_SESSION="$(login maria "$MARIA_PASSWORD")"
[ -n "$MARIA_SESSION" ] && [ "$MARIA_SESSION" != "null" ] || fail "maria's login failed"
redact "$MARIA_SESSION" "<maria's session token>"
request "$OPS" "04-effect" GET /auth/me maria ""
expect "operator/04-effect" 200 '.roles == ["releaser"]' '.maintains == ["payments"]' '.is_root == false'

# 05 — the webhook allow-list
printf '{"hosts":["hooks.slack.com","http://127.0.0.1"]}' >"$TMP/body.json"
request "$OPS" "05-allow-list" POST /admin/settings/webhook-allowed-hosts session "$TMP/body.json"
expect "operator/05" 200 '.hosts == ["hooks.slack.com","http://127.0.0.1"]'

# 06 — the allow-list binding a maintainer; a Slack-format hook by root (exempt),
#      a test delivery, the delivery log, the signature checked
printf '{"url":"https://example.com/sanshain","format":"json"}' >"$TMP/body.json"
request "$OPS" "06-maintainer-refused" POST /admin/producers/payments/webhooks maria "$TMP/body.json"
expect "operator/06-maintainer-refused" 403 '.error | test("allow-list")'
printf '{"url":"http://127.0.0.1:%s/sanshain","format":"slack"}' "$RECEIVER_PORT" >"$TMP/body.json"
request "$OPS" "06-create-hook" POST /admin/webhooks session "$TMP/body.json"
expect "operator/06-create" 201 '.secret | startswith("whsec_")' '.webhook.format == "slack"' '.webhook.allow_list_exempt == true'
HOOK_ID="$(jq -r .webhook.id "$LAST_BODY")"
HOOK_SECRET="$(jq -r .secret "$LAST_BODY")"
# The signing secret is shown once; it must not land in the repository either (secret
# scanners match the whsec_ prefix), so every evidence file carries the placeholder.
redact "$HOOK_SECRET" "whsec_…"
request "$OPS" "06-test-hook" POST "/admin/webhooks/$HOOK_ID/test" session ""
expect "operator/06-test" 202 '.event_id | length == 36'
for _ in $(seq 1 30); do
  [ -f "$TMP/deliveries/1.json" ] && break
  sleep 1
done
[ -f "$TMP/deliveries/1.json" ] || fail "operator/06: the receiver got no delivery within 30 s"
{
  jq -r '"\(.method) \(.url) HTTP/1.1"' "$TMP/deliveries/1.json"
  jq -r '.headers | to_entries[] | select(.key | test("^(content-type|user-agent|sanshain-event|x-sanshain-delivery|sanshain-signature)$")) | "\(.key): \(.value)"' "$TMP/deliveries/1.json" \
    | sed -E 's/^content-type/Content-Type/;s/^user-agent/User-Agent/;s/^sanshain-event/Sanshain-Event/;s/^x-sanshain-delivery/X-Sanshain-Delivery/;s/^sanshain-signature/Sanshain-Signature/'
  echo
  jq -r .body "$TMP/deliveries/1.json" | jq .
  echo
  echo "HTTP/1.1 200 OK"
} >"$OPS/06-delivery-received.http"
for _ in $(seq 1 20); do
  request "$OPS" "06-deliveries" GET "/admin/webhooks/$HOOK_ID/deliveries?limit=5" session "" >/dev/null
  if jq -e '.[0].status == "delivered"' "$LAST_BODY" >/dev/null 2>&1; then break; fi
  sleep 1
done
expect "operator/06-deliveries" 200 '.[0].status == "delivered"' '.[0].event_type == "webhook.test"'
cat >"$TMP/verify.py" <<'EOF'
import hashlib, hmac, json, sys
delivery = json.load(open(sys.argv[1]))
secret = sys.argv[2]
parts = dict(p.split("=", 1) for p in delivery["headers"]["sanshain-signature"].split(","))
expected = hmac.new(secret.encode(), f"{parts['t']}.".encode() + delivery["body"].encode(), hashlib.sha256).hexdigest()
print("signature header:", delivery["headers"]["sanshain-signature"])
print("secret from the create answer:", secret)
print("HMAC-SHA256 of '<t>.<raw body>':", expected)
print("valid:", hmac.compare_digest(expected, parts["v1"]))
sys.exit(0 if hmac.compare_digest(expected, parts["v1"]) else 1)
EOF
SHOW="python3 verify.py delivery.json $HOOK_SECRET" \
  transcript "$OPS" "06-verify-signature" "The receiver's check, with the secret shown once at creation" \
  python3 "$TMP/verify.py" "$TMP/deliveries/1.json" "$HOOK_SECRET"

# 07 — retention for snapshots, and a cleanup run
printf '{"days":30}' >"$TMP/body.json"
request "$OPS" "07-retention" POST /admin/settings/snapshot-max-age session "$TMP/body.json"
expect "operator/07-retention" 200
expect_empty "operator/07-retention"
request "$OPS" "07-retention-state" GET /admin/settings/snapshot-max-age session ""
expect "operator/07-retention-state" 200 '[.. | numbers] | index(30) != null'
printf '{}' >"$TMP/body.json"
request "$OPS" "07-cleanup" POST /admin/cleanup/snapshots session "$TMP/body.json"
expect "operator/07-cleanup" 200 '.deleted == 0'

# 08 — the SQLite snapshot, as the CronJob fetches it; the restore, as documented
request "$OPS" "08-snapshot" GET /admin/backup/sqlite token ""
expect "operator/08-snapshot" 200
grep -qi '^content-type: application/vnd.sqlite3' "$LAST_HEADERS" || fail "operator/08: expected a SQLite body"
[ "$(stat -c %s "$LAST_BODY")" -gt 65536 ] || fail "operator/08: the snapshot is implausibly small"
[ "$(head -c 15 "$LAST_BODY")" = "SQLite format 3" ] || fail "operator/08: the body is not a SQLite file"
SNAPSHOT_NAME="$(grep -i '^content-disposition:' "$LAST_HEADERS" | sed -E 's/.*filename="([^"]+)".*/\1/' | tr -d '\r')"
cat >"$OPS/08-restore.txt" <<EOF
# Not recorded against a cluster: the restore procedure from Backup and restore,
# with the snapshot the CronJob fetched above (${SNAPSHOT_NAME:-sanshain-backup.db}).
\$ kubectl scale statefulset sanshain --replicas=0
statefulset.apps/sanshain scaled
\$ kubectl run restore --rm -it --image=busybox --overrides='{"spec":{"volumes":[{"name":"data","persistentVolumeClaim":{"claimName":"sanshain-data"}},{"name":"backup","persistentVolumeClaim":{"claimName":"sanshain-backup"}}],"containers":[{"name":"restore","image":"busybox","command":["sh"],"stdin":true,"tty":true,"volumeMounts":[{"name":"data","mountPath":"/data"},{"name":"backup","mountPath":"/backup"}]}]}}' -- sh
/ # cp /backup/${SNAPSHOT_NAME:-sanshain-backup.db} /data/sanshain.db
/ # rm -f /data/sanshain.db-wal /data/sanshain.db-shm
/ # exit
\$ kubectl scale statefulset sanshain --replicas=1
statefulset.apps/sanshain scaled
\$ kubectl logs statefulset/sanshain | grep -i migrations
INFO sanshain_service::infrastructure::sqlite_repository: Running SQLite migrations...
EOF

# 09 — the audit log after all of the above
request "$OPS" "09-audit-log" GET "/admin/observability/audit-logs?limit=12" session ""
expect "operator/09" 200 'length >= 10' '.[0].action == "DATABASE_SNAPSHOT"' 'map(.action) | index("SET_WEBHOOK_ALLOWED_HOSTS") != null'

# 10 — the metrics exposition
METRICS="$(curl -sS "$BASE/metrics" | { grep '^sanshain_' || true; } | sort)"
[ -n "$METRICS" ] || fail "operator/10: /metrics answered no sanshain_ lines"
echo "$METRICS" | grep -q '^sanshain_version_rejected_total' || fail "operator/10: sanshain_version_rejected_total is missing"
{
  echo "\$ curl -s $PUBLIC_BASE_URL/metrics | grep '^sanshain_'"
  echo "$METRICS"
} >"$OPS/10-metrics.txt"

# ---------------------------------------------------------------------------
# UI steps and chapter screenshots

say "Screenshots (Playwright)"
shots --grep-invert "developer 03"

say "Recorded $(find "$EVIDENCE" -type f | wc -l) evidence files under site/evidence/"
