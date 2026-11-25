#!/usr/bin/env bash
# Builds dist/perusal-chromium.zip and dist/perusal-firefox.zip from one codebase.
set -euo pipefail
cd "$(dirname "$0")"; rm -rf dist; mkdir -p dist/chromium dist/firefox
for t in chromium firefox; do cp detect.js content.js options.html options.js dist/$t/; done
cp manifest.base.json dist/chromium/manifest.json
node -e '
const m = require("./manifest.base.json");
m.browser_specific_settings = { gecko: { id: "perusal@example.invalid", strict_min_version: "115.0" } };
m.background = undefined;
require("fs").writeFileSync("dist/firefox/manifest.json", JSON.stringify(m, null, 2));'
for t in chromium firefox; do (cd dist/$t && zip -qr ../perusal-$t.zip .); done
echo "built: $(ls dist/*.zip | tr '\n' ' ')"
