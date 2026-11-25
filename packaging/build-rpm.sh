#!/usr/bin/env bash
# Needs rpmbuild. Copies the release artifacts into a temp rpmbuild tree and builds.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v rpmbuild >/dev/null || { echo "rpmbuild not installed; skipping rpm"; exit 0; }
cargo build --release -p perusal-ui -p perusal-cli
T=$(mktemp -d); mkdir -p $T/SOURCES
cp target/release/perusal target/release/perusal-cli packaging/perusal.desktop assets/perusal.svg $T/SOURCES/
V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2); V=${V//-/\~}   # rpm versions cannot contain '-'
rpmbuild --define "_topdir $T" --define "pversion $V" -bb packaging/perusal.spec && find $T/RPMS -name '*.rpm' -exec cp {} target/ \;
