#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
command -v flatpak-builder >/dev/null || { echo "flatpak-builder not installed; manifest is io.github.perusal.Perusal.yml"; exit 0; }
(cd .. && cargo build --release -p perusal-ui)
flatpak-builder --force-clean ../target/flatpak-build io.github.perusal.Perusal.yml
