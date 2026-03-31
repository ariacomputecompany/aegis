#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../web-ui"

if [ ! -d node_modules ]; then
  npm install
fi

npx tsc -b
npx vite build
echo "Web UI built to web-ui/dist/"
