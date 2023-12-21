#!/usr/bin/env bash

# Previous auth token format
read -r -d '' CONFIG <<- EOF
{
  "token": "normal-user-token"
}
EOF

# New auth token format
read -r -d '' AUTH <<- EOF
{
  "tokens": {
    "vercel.com/api": "normal vercel token"
  }
}
EOF

TMP_DIR=$(mktemp -d -t turbo-XXXXXXXXXX)
export TURBO_CONFIG_DIR_PATH=$TMP_DIR

# duplicate over to XDG var so that turbo picks it up
export XDG_CONFIG_HOME=$TMP_DIR
export HOME=$TMP_DIR

# For Linux
mkdir -p "$TMP_DIR/turborepo"
echo $CONFIG > "$TMP_DIR/turborepo/config.json"
echo $AUTH > "$TMP_DIR/turborepo/auth.json"

# For macOS
MACOS_DIR="$TMP_DIR/Library/Application Support"
mkdir -p "$MACOS_DIR/turborepo"
echo "$CONFIG" > "$MACOS_DIR/turborepo/config.json"
echo "$AUTH" > "$MACOS_DIR/turborepo/auth.json"

# XDG_CONFIG_HOME equivalent for Windows is {FOLDERID_RoamingAppData} which is roughly C:\Users\{username}\AppData\Roaming
WINDOWS_DIR="$TMP_DIR/AppData/Roaming"
mkdir -p "$WINDOWS_DIR/turborepo"
echo "$CONFIG" > "$WINDOWS_DIR/turborepo/config.json"
echo "$AUTH" > "$WINDOWS_DIR/turborepo/auth.json"
