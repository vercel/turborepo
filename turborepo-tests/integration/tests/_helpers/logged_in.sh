#!/usr/bin/env bash

read -r -d '' CONFIG <<- EOF
{
  "vercel.com/api": "normal-user-token"
}
EOF

TMP_DIR=$(mktemp -d -t turbo-XXXXXXXXXX)

# duplicate over to XDG var so that turbo picks it up
export XDG_CONFIG_HOME=$TMP_DIR
export HOME=$TMP_DIR

# For Linux
mkdir -p "$TMP_DIR/turborepo"
echo $CONFIG > "$TMP_DIR/turborepo/config.json"

# For macOS
MACOS_DIR="$TMP_DIR/Library/Application Support"
mkdir -p "$MACOS_DIR/turborepo"
echo "$CONFIG" > "$MACOS_DIR/turborepo/config.json"

# XDG_CONFIG_HOME equivalent for Windows is {FOLDERID_RoamingAppData} which is roughly C:\Users\{username}\AppData\Roaming
WINDOWS_DIR="$TMP_DIR/AppData/Roaming"
mkdir -p "$WINDOWS_DIR/turborepo"
echo "$CONFIG" > "$WINDOWS_DIR/turborepo/config.json"
