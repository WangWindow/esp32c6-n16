#!/usr/bin/env bash
# Build, flash and run one example in release mode.
#
# Usage:
#   scripts/run-example.sh <selftest|wifi|ble>
set -euo pipefail

cd "$(dirname "$0")/.."

EXAMPLE="${1:-}"
case "$EXAMPLE" in
    selftest|wifi|ble) ;;
    *)
        echo "usage: scripts/run-example.sh <selftest|wifi|ble>" >&2
        exit 2
        ;;
esac

if [[ -f .env ]]; then
    set -a
    # shellcheck disable=SC1091
    source .env
    set +a
fi

exec cargo run -p "esp32c6-n16-example-$EXAMPLE" --release
