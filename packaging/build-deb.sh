#!/usr/bin/env bash
# Builds target/perusal_<version>_amd64.deb from the release binary.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p perusal-app -p perusal-cli
V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2); R=target/deb-root; rm -rf $R
install -Dm755 target/release/perusal $R/usr/bin/perusal
install -Dm755 target/release/perusal-cli $R/usr/bin/perusal-cli
install -Dm644 packaging/perusal.desktop $R/usr/share/applications/perusal.desktop
install -Dm644 crates/app/themes/agents.md $R/usr/share/perusal/themes/agents.md
install -Dm644 skills/review.md $R/usr/share/perusal/skills/review.md
install -Dm644 LICENSE-MIT $R/usr/share/doc/perusal/LICENSE-MIT
install -Dm644 LICENSE-APACHE $R/usr/share/doc/perusal/LICENSE-APACHE
mkdir -p $R/DEBIAN
cat > $R/DEBIAN/control <<C
Package: perusal
Version: $V
Architecture: amd64
Maintainer: Perusal contributors <noreply@example.invalid>
Depends: git
Description: Fast merge-request and pull-request reviewer
 Filter, batch and graph based review for GitLab and GitHub.
C
dpkg-deb --build --root-owner-group $R target/perusal_${V}_amd64.deb
