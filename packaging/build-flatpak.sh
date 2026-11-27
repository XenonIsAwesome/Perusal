#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
command -v flatpak-builder >/dev/null || { echo "flatpak-builder not installed; manifest is io.github.perusal.Perusal.yml"; exit 0; }
(cd .. && cargo build --release -p perusal-ui)
# --user installs only see user remotes, so make sure flathub is one (a system-wide flathub remote is not enough)
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
# start from a fresh ostree repo: a stale or half-created one restored from a build cache fails with "opening repo: openat(config)"
rm -rf ../target/flatpak-repo
flatpak-builder --force-clean --user --install-deps-from=flathub --repo=../target/flatpak-repo ../target/flatpak-build io.github.perusal.Perusal.yml
flatpak build-bundle ../target/flatpak-repo ../target/perusal.flatpak io.github.perusal.Perusal
