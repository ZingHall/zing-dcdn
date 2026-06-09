#!/bin/bash
# Build frontend, then run Tauri dev.
# Usage: ./dev.sh                          (default ports)
#        ZING_P2P_PORT=34292 ZING_API_PORT=13421 ZING_CACHE_DIR=/tmp/zing-cache-B ./dev.sh

set -e

dx build

echo "Frontend built. Starting Tauri dev..."

exec cargo tauri dev
