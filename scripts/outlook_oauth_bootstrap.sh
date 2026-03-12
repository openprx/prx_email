#!/usr/bin/env bash
set -euo pipefail

# Outlook OAuth2 bootstrap helper for IMAP/SMTP.
# - Generates authorize URL
# - Accepts callback URL or raw code
# - Exchanges auth code for access + refresh token
# - Saves tokens to local env file (0600)

usage() {
  cat <<'EOF'
Usage:
  CLIENT_ID=... TENANT=... REDIRECT_URI=... ./scripts/outlook_oauth_bootstrap.sh [--output <file>] [--dry-run]

Required inputs (env vars):
  CLIENT_ID      Azure App (client) ID
  TENANT         Tenant ID or 'common'/'organizations'/'consumers'
  REDIRECT_URI   Redirect URI registered in Azure app

Optional env vars:
  SCOPE          Space-separated scopes override (default includes IMAP/SMTP + offline_access)

Flags:
  --output <file>  Write env output to this file (default: ./outlook_oauth.local.env)
  --dry-run        Only print authorize URL and exit (no token exchange)
  -h, --help       Show this help
EOF
}

err() { echo "[ERROR] $*" >&2; }
info() { echo "[INFO]  $*"; }
warn() { echo "[WARN]  $*" >&2; }

require_cmd() {
  local c="$1"
  command -v "$c" >/dev/null 2>&1 || {
    err "Missing required command: $c"
    exit 1
  }
}

urlencode() {
  local raw="$1"
  if command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY' "$raw"
import sys, urllib.parse
print(urllib.parse.quote(sys.argv[1], safe=''))
PY
  elif command -v perl >/dev/null 2>&1; then
    perl -MURI::Escape -e 'print uri_escape($ARGV[0]);' "$raw"
  else
    err "Need python3 or perl for URL encoding"
    exit 1
  fi
}

json_get() {
  local key="$1" file="$2"
  if command -v jq >/dev/null 2>&1; then
    jq -r --arg k "$key" '.[$k] // empty' "$file"
  elif command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY' "$key" "$file"
import json,sys
k,p = sys.argv[1],sys.argv[2]
with open(p,'r',encoding='utf-8') as f:
  data=json.load(f)
v=data.get(k,"")
print(v if v is not None else "")
PY
  else
    err "Need jq or python3 to parse token response JSON"
    exit 1
  fi
}

extract_code() {
  local input="$1"
  if [[ "$input" =~ ^https?:// ]]; then
    if [[ "$input" == *"code="* ]]; then
      if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY' "$input"
import sys, urllib.parse
u = urllib.parse.urlparse(sys.argv[1])
q = urllib.parse.parse_qs(u.query)
print(q.get('code', [''])[0])
PY
      else
        echo "$input" | sed -n 's/.*[?&]code=\([^&]*\).*/\1/p'
      fi
    else
      echo ""
    fi
  else
    # Assume caller pasted raw code.
    echo "$input"
  fi
}

mask_token() {
  local t="$1"
  local n=${#t}
  if (( n <= 12 )); then
    echo "***"
  else
    echo "${t:0:6}...${t:n-4:4}"
  fi
}

OUT_FILE="./outlook_oauth.local.env"
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      [[ $# -ge 2 ]] || { err "--output requires a file path"; exit 1; }
      OUT_FILE="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      err "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
done

require_cmd curl

: "${CLIENT_ID:?CLIENT_ID is required}"
: "${TENANT:?TENANT is required}"
: "${REDIRECT_URI:?REDIRECT_URI is required}"

SCOPE_DEFAULT="offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send"
SCOPE="${SCOPE:-$SCOPE_DEFAULT}"

AUTH_BASE="https://login.microsoftonline.com/${TENANT}/oauth2/v2.0"
AUTH_URL="${AUTH_BASE}/authorize?client_id=$(urlencode "$CLIENT_ID")&response_type=code&redirect_uri=$(urlencode "$REDIRECT_URI")&response_mode=query&scope=$(urlencode "$SCOPE")"
TOKEN_URL="${AUTH_BASE}/token"

info "Open this URL in your browser and complete login/consent:"
echo "$AUTH_URL"

if [[ "$DRY_RUN" -eq 1 ]]; then
  info "Dry-run mode enabled. Exiting before token exchange."
  exit 0
fi

echo
read -r -p "Paste callback URL (or raw authorization code): " CALLBACK_INPUT
CODE="$(extract_code "$CALLBACK_INPUT")"
if [[ -z "${CODE// }" ]]; then
  err "Could not extract authorization code. Paste full callback URL or raw code."
  exit 1
fi

TMP_JSON="$(mktemp)"
trap 'rm -f "$TMP_JSON"' EXIT

HTTP_CODE="$({
  curl -sS -o "$TMP_JSON" -w '%{http_code}' -X POST "$TOKEN_URL" \
    -H 'Content-Type: application/x-www-form-urlencoded' \
    --data-urlencode "client_id=${CLIENT_ID}" \
    --data-urlencode "grant_type=authorization_code" \
    --data-urlencode "code=${CODE}" \
    --data-urlencode "redirect_uri=${REDIRECT_URI}"
} || true)"

if [[ "$HTTP_CODE" != "200" ]]; then
  err "Token exchange failed (HTTP ${HTTP_CODE:-unknown})."
  if [[ -s "$TMP_JSON" ]]; then
    warn "Response body:"
    cat "$TMP_JSON" >&2
  fi
  exit 1
fi

ACCESS_TOKEN="$(json_get access_token "$TMP_JSON")"
REFRESH_TOKEN="$(json_get refresh_token "$TMP_JSON")"
EXPIRES_IN="$(json_get expires_in "$TMP_JSON")"
SCOPE_GRANTED="$(json_get scope "$TMP_JSON")"
TOKEN_TYPE="$(json_get token_type "$TMP_JSON")"

if [[ -z "$ACCESS_TOKEN" || -z "$REFRESH_TOKEN" ]]; then
  err "Missing access_token or refresh_token in token response."
  warn "Raw response:"
  cat "$TMP_JSON" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT_FILE")"
{
  echo "# Generated by scripts/outlook_oauth_bootstrap.sh on $(date -u +'%Y-%m-%dT%H:%M:%SZ')"
  echo "# DO NOT COMMIT this file. Contains live OAuth credentials."
  echo "OUTLOOK_CLIENT_ID=${CLIENT_ID}"
  echo "OUTLOOK_TENANT=${TENANT}"
  echo "OUTLOOK_REDIRECT_URI=${REDIRECT_URI}"
  echo "OUTLOOK_SCOPE=${SCOPE_GRANTED:-$SCOPE}"
  echo "OUTLOOK_TOKEN_TYPE=${TOKEN_TYPE:-Bearer}"
  echo "OUTLOOK_ACCESS_TOKEN=${ACCESS_TOKEN}"
  echo "OUTLOOK_REFRESH_TOKEN=${REFRESH_TOKEN}"
  echo "OUTLOOK_EXPIRES_IN=${EXPIRES_IN:-0}"
} > "$OUT_FILE"

chmod 600 "$OUT_FILE"

info "Saved OAuth tokens to: $OUT_FILE (chmod 600)"
info "Access token : $(mask_token "$ACCESS_TOKEN")"
info "Refresh token: $(mask_token "$REFRESH_TOKEN")"
info "Done."
