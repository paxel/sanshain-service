#!/usr/bin/env bash
# The Tailwind CSS is a committed artifact (prod serves static/ directly, with
# no frontend build). This fails if static/css/app.css is stale relative to the
# source it is generated from — run `npm run build:css` and commit the result.
set -euo pipefail
cd "$(dirname "$0")/.."
TW="../node_modules/.bin/tailwindcss"
[ -x "$TW" ] || TW="node_modules/.bin/tailwindcss"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
"$TW" -c tailwind.config.js -i static/css/app.input.css -o "$TMP" --minify >/dev/null 2>&1
if ! diff -q "$TMP" static/css/app.css >/dev/null; then
  echo "static/css/app.css is stale — run 'npm run build:css' and commit it." >&2
  exit 1
fi
echo "app.css is up to date."
