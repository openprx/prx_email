#!/usr/bin/env bash
set -euo pipefail

# Required env vars:
#   E2E_IMAP_HOST, E2E_IMAP_PORT, E2E_SMTP_HOST, E2E_SMTP_PORT,
#   E2E_EMAIL_USER, E2E_EMAIL_PASS
# Optional:
#   E2E_TARGET_EMAIL (defaults to E2E_EMAIL_USER)

: "${E2E_IMAP_HOST:?missing E2E_IMAP_HOST}"
: "${E2E_SMTP_HOST:?missing E2E_SMTP_HOST}"
: "${E2E_EMAIL_USER:?missing E2E_EMAIL_USER}"
: "${E2E_EMAIL_PASS:?missing E2E_EMAIL_PASS}"

export E2E_IMAP_PORT="${E2E_IMAP_PORT:-993}"
export E2E_SMTP_PORT="${E2E_SMTP_PORT:-465}"
export E2E_TARGET_EMAIL="${E2E_TARGET_EMAIL:-$E2E_EMAIL_USER}"

source ~/.cargo/env
cargo test --test m4_e2e_smoke -- --ignored --nocapture
