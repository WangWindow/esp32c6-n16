#!/usr/bin/env bash
# Static checks for the app and all examples. Uses release profiles only.
set -euo pipefail

cd "$(dirname "$0")/.."

PACKAGES=(
    esp32c6-n16-app
    esp32c6-n16-example-selftest
    esp32c6-n16-example-wifi
    esp32c6-n16-example-ble
)

echo "==> cargo fmt"
cargo fmt --all --check

for package in "${PACKAGES[@]}"; do
    echo "==> clippy: $package"
    cargo clippy -p "$package" --release -- -D warnings

    echo "==> build: $package"
    cargo build -p "$package" --release
done

echo "==> compile on-target tests"
cargo test -p esp32c6-n16-app --release --test hardware --no-run

echo
echo "All static checks passed."
echo "Run hardware tests with:"
echo "  cargo test -p esp32c6-n16-app --release --test hardware"
