#!/usr/bin/env bash
# Needs appimagetool on PATH. Builds an AppDir; first run offers to install the .desktop entry (see AppRun).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p perusal-app
A=target/Perusal.AppDir; rm -rf $A; mkdir -p $A/usr/bin
cp target/release/perusal $A/usr/bin/; cp packaging/perusal.desktop $A/
cat > $A/AppRun <<'R'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
D="${XDG_DATA_HOME:-$HOME/.local/share}/applications/perusal.desktop"
if [ ! -f "$D" ] && [ -t 0 ]; then
  printf 'Register perusal:// links for this AppImage? [y/N] '; read a
  if [ "$a" = y ]; then mkdir -p "$(dirname "$D")"; sed "s|^Exec=.*|Exec=$APPIMAGE %u|" "$HERE/perusal.desktop" > "$D"; fi
fi
exec "$HERE/usr/bin/perusal" "$@"
R
chmod +x $A/AppRun
command -v appimagetool >/dev/null || { echo "appimagetool not installed; AppDir left at $A"; exit 0; }
appimagetool $A target/Perusal-x86_64.AppImage
