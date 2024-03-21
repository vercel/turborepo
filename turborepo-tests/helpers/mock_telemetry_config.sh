#!/usr/bin/env bash

read -r -d '' CONFIG <<- EOF
{
  "telemetry_enabled": true
}
EOF

TMP_DIR=$(mktemp -d -t turbo-XXXXXXXXXX)

# duplicate over to XDG var so that turbo picks it up
export XDG_CONFIG_HOME=$TMP_DIR
export HOME=$TMP_DIR
export TURBO_TELEMETRY_MESSAGE_DISABLED=1
export TURBO_CONFIG_DIR_PATH=$TMP_DIR

# For Linux
mkdir -p "$TMP_DIR/turborepo"
echo $CONFIG > "$TMP_DIR/turborepo/telemetry_config.json"

# For macOS
MACOS_DIR="$TMP_DIR/Library/Application Support"
mkdir -p "$MACOS_DIR/turborepo"
echo "$CONFIG" > "$MACOS_DIR/turborepo/telemetry_config.json"
