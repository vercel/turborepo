#!/usr/bin/env bash

# Create a mocked Vercel auth file
read -r -d  '' AUTH <<- EOF
{
  "// Note": "This is your Vercel credentials file. DO NOT SHARE!",
  "// Docs": "https://vercel.com/docs/project-configuration#global-configuration/auth-json",
  "token": "mock-token"
}
EOF

TMP_DIR=$(mktemp -d -t turbo-XXXXXXXXXX)

# duplicate over to XDG var so that turbo picks it up
export XDG_CONFIG_HOME=$TMP_DIR
export HOME=$TMP_DIR
export TURBO_CONFIG_DIR_PATH=$TMP_DIR
export VERCEL_CONFIG_DIR_PATH=$TMP_DIR
export TURBO_TELEMETRY_MESSAGE_DISABLED=1

# For Linux
mkdir -p "$TMP_DIR/com.vercel.cli"
echo $AUTH > "$TMP_DIR/com.vercel.cli/auth.json"

# For macOS
MACOS_DIR="$TMP_DIR/Library/Application Support"
mkdir -p "$MACOS_DIR/com.vercel.cli"
echo "$AUTH" > "$MACOS_DIR/com.vercel.cli/auth.json"

# For Windows
WINDOWS_DIR="$TMP_DIR\\AppData\\Roaming"
mkdir -p "$WINDOWS_DIR\\com.vercel.cli"
echo "$AUTH" > "$WINDOWS_DIR\\com.vercel.cli\\auth.json"
