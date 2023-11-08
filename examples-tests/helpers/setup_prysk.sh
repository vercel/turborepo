#!/bin/bash

set -e
BASE_DIR="$PWD/.."

echo "basedir: $BASE_DIR"

if [ -f "$BASE_DIR/.cram_env/bin/prysk" ]; then
  echo "Skipping prysk setup, prysk and venv already exists"
else
  python3 -m venv "$BASE_DIR/.cram_env"
  "$BASE_DIR/.cram_env/bin/python3" -m pip install --quiet --upgrade pip
  "$BASE_DIR/.cram_env/bin/pip" install "prysk==0.15.0"
fi
