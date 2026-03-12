#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGIN_DIR="${ROOT_DIR}/wasm-plugin"

source ~/.cargo/env
cd "${PLUGIN_DIR}"

cargo component build --release

WASIP2_PATH="target/wasm32-wasip2/release/prx_email_plugin.wasm"
WASIP1_PATH="target/wasm32-wasip1/release/prx_email_plugin.wasm"
OUT_WASM=""

if [[ -f "$WASIP2_PATH" && -f "$WASIP1_PATH" ]]; then
  if [[ "$WASIP2_PATH" -nt "$WASIP1_PATH" ]]; then
    OUT_WASM="$WASIP2_PATH"
  else
    OUT_WASM="$WASIP1_PATH"
  fi
elif [[ -f "$WASIP2_PATH" ]]; then
  OUT_WASM="$WASIP2_PATH"
elif [[ -f "$WASIP1_PATH" ]]; then
  OUT_WASM="$WASIP1_PATH"
else
  echo "cannot find built wasm artifact" >&2
  exit 1
fi

cp "$OUT_WASM" "plugin.wasm"
echo "built: ${PLUGIN_DIR}/plugin.wasm (from $OUT_WASM)"
