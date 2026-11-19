#!/usr/bin/env bash
# Build, flash and run the main application in release mode.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -f .env ]]; then
    set -a
    # shellcheck disable=SC1091
    source .env
    set +a
fi

exec cargo run -p esp32c6-n16-app --release
