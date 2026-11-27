#!/usr/bin/env bash
# Needs appimagetool on PATH. Builds an AppDir; first run offers to install the .desktop entry (see AppRun).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p perusal-ui
A=target/Perusal.AppDir; rm -rf $A; mkdir -p $A/usr/bin
cp target/release/perusal $A/usr/bin/
# the desktop file id must match the AppStream component id: appimagetool validates the metainfo and warnings fail the build.
# appimagetool only looks for <desktop id>.appdata.xml (not .metainfo.xml) under usr/share/metainfo
install -Dm644 packaging/perusal.desktop $A/io.github.perusal.Perusal.desktop
install -Dm644 packaging/perusal.desktop $A/usr/share/applications/io.github.perusal.Perusal.desktop
# icon: appimagetool wants <Icon=name>.png at the AppDir root and makes .DirIcon (must be a PNG) from it
cp crates/ui/assets/perusal-256.png $A/perusal.png
install -Dm644 packaging/io.github.perusal.Perusal.metainfo.xml $A/usr/share/metainfo/io.github.perusal.Perusal.appdata.xml
install -Dm644 assets/perusal.svg $A/usr/share/icons/hicolor/scalable/apps/perusal.svg
cat > $A/AppRun <<'R'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
D="${XDG_DATA_HOME:-$HOME/.local/share}/applications/perusal.desktop"
if [ ! -f "$D" ] && [ -t 0 ]; then
  printf 'Register perusal:// links for this AppImage? [y/N] '; read a
  if [ "$a" = y ]; then mkdir -p "$(dirname "$D")"; sed "s|^Exec=.*|Exec=$APPIMAGE %u|" "$HERE/io.github.perusal.Perusal.desktop" > "$D"; fi
fi
exec "$HERE/usr/bin/perusal" "$@"
R
chmod +x $A/AppRun
command -v appimagetool >/dev/null || { echo "appimagetool not installed; AppDir left at $A"; exit 0; }
ARCH=x86_64 appimagetool --appimage-extract-and-run $A target/Perusal-x86_64.AppImage
