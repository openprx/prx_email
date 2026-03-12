#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGIN_DIR="${ROOT_DIR}/wasm-plugin"

source ~/.cargo/env
cd "${PLUGIN_DIR}"

cargo component build --release

OUT_WASM=""
if [[ -f "target/wasm32-wasip2/release/prx_email_plugin.wasm" ]]; then
  OUT_WASM="target/wasm32-wasip2/release/prx_email_plugin.wasm"
elif [[ -f "target/wasm32-wasip1/release/prx_email_plugin.wasm" ]]; then
  OUT_WASM="target/wasm32-wasip1/release/prx_email_plugin.wasm"
else
  echo "cannot find built wasm artifact" >&2
  exit 1
fi

cp "$OUT_WASM" "plugin.wasm"
echo "built: ${PLUGIN_DIR}/plugin.wasm (from $OUT_WASM)"
