#!/usr/bin/env bash
set -euo pipefail

ORIGIN="${ORIGIN:-}"
API_BASE="${API_BASE:-}"
WRITER_TOKEN="${WRITER_TOKEN:-}"
APP_SLUG="${APP_SLUG:-pdes-fwx-updater}"
TARGET="${TARGET:-windows:x86_64}"
EXPIRES_IN_SECONDS="${EXPIRES_IN_SECONDS:-3600}"
MAX_USES="${MAX_USES:-10}"

usage() {
  cat <<EOF
Usage:
  WRITER_TOKEN=... ORIGIN=https://YOUR_HOST \\
    $0 [--app-slug SLUG] [--target windows:x86_64] [--expires 3600] [--max-uses 10]

Env:
  WRITER_TOKEN (required)
  ORIGIN (required, e.g. https://YOUR_HOST)
  API_BASE (optional, defaults to ORIGIN/api)
  APP_SLUG (default: pdes-fwx-updater)
  TARGET (default: windows:x86_64)
  EXPIRES_IN_SECONDS (default: 3600)
  MAX_USES (default: 10, minimum enforced: 2)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --origin) ORIGIN="$2"; shift 2 ;;
    --api-base) API_BASE="$2"; shift 2 ;;
    --app-slug) APP_SLUG="$2"; shift 2 ;;
    --target) TARGET="$2"; shift 2 ;;
    --expires|--expires-in-seconds) EXPIRES_IN_SECONDS="$2"; shift 2 ;;
    --max-uses) MAX_USES="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "$WRITER_TOKEN" ]]; then
  echo "WRITER_TOKEN is required" >&2
  exit 2
fi
if [[ -z "$ORIGIN" ]]; then
  echo "ORIGIN is required (e.g. https://YOUR_HOST)" >&2
  exit 2
fi

ORIGIN="${ORIGIN%/}"
if [[ -z "$API_BASE" ]]; then
  API_BASE="${ORIGIN}/api"
fi
API_BASE="${API_BASE%/}"

if ! [[ "$MAX_USES" =~ ^[0-9]+$ ]]; then
  echo "MAX_USES must be an integer" >&2
  exit 2
fi
if (( MAX_USES < 2 )); then
  MAX_USES=2
fi

payload="$(printf '{"app_slug":"%s","target":"%s","expires_in_seconds":%s,"max_uses":%s}' \
  "$APP_SLUG" "$TARGET" "$EXPIRES_IN_SECONDS" "$MAX_USES")"

resp="$(
  curl -sfS \
    -H "Authorization: Bearer ${WRITER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "$payload" \
    "${API_BASE}/bootstrap/mint"
)"

extract() {
  local json="$1"
  local key="$2"
  if command -v jq >/dev/null 2>&1; then
    jq -r "$key // empty" <<<"$json"
  else
    python3 - <<'PY' "$json" "$key"
import json, sys
obj = json.loads(sys.argv[1])
key = sys.argv[2]
def get(o, path):
    cur = o
    for p in path.split("."):
        if not isinstance(cur, dict) or p not in cur:
            return ""
        cur = cur[p]
    return cur if isinstance(cur, str) else ""
print(get(obj, key))
PY
  fi
}

pkg_url="$(extract "$resp" "bootstrap_pkg_url")"
token="$(extract "$resp" "bootstrap_token")"
if [[ -z "$pkg_url" && -n "$token" ]]; then
  pkg_url="${ORIGIN}/bootstrap/${token}/pkg"
fi

if [[ -z "$pkg_url" ]]; then
  echo "Unable to determine bootstrap package URL from response:" >&2
  echo "$resp" >&2
  exit 1
fi

echo "$pkg_url"
