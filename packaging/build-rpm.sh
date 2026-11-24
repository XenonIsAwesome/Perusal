#!/usr/bin/env bash
# Needs rpmbuild. Copies the release artifacts into a temp rpmbuild tree and builds.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v rpmbuild >/dev/null || { echo "rpmbuild not installed; skipping rpm"; exit 0; }
cargo build --release -p perusal-app -p perusal-cli
T=$(mktemp -d); mkdir -p $T/SOURCES
cp target/release/perusal target/release/perusal-cli packaging/perusal.desktop $T/SOURCES/
rpmbuild --define "_topdir $T" -bb packaging/perusal.spec && find $T/RPMS -name '*.rpm' -exec cp {} target/ \;
