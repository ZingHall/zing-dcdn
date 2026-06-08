#!/bin/bash
# Build frontend with custom API port, then run Tauri dev.
# Usage: ./dev.sh                          (default port 13420)
#        ZING_API_PORT=13421 ./dev.sh      (custom port)
#        ZING_P2P_PORT=34292 ZING_API_PORT=13421 ./dev.sh

PORT=${ZING_API_PORT:-13420}

set -e

# Build frontend
dx build

# Patch module script (remove async) + inject API port
sed -i '' \
  -e 's/ type="module" async / type="module" /' \
  -e "s/data-api-port=\"13420\"/data-api-port=\"$PORT\"/" \
  target/dx/zing_cdn_gui/debug/web/public/index.html

echo "Frontend built with API port $PORT"

# Run Tauri dev (skips beforeDevCommand since files are already patched)
exec cargo tauri dev
