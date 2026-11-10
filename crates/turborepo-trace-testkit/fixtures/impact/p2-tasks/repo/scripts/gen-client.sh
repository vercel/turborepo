#!/usr/bin/env bash
# Generates src/generated/client.js (`npm run gen:client`).
set -euo pipefail
source "$(dirname "$0")/../tools/sh/env.sh"
node "$(dirname "$0")/gen-client.js" > "$(dirname "$0")/../src/generated/client.js"
