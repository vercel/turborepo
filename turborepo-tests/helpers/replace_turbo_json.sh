#!/usr/bin/env bash

PROJECT_DIR=$1
CONFIG_NAME=$2

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."

TURBO_CONFIGS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests/integration/fixtures/turbo-configs"

cp "${TURBO_CONFIGS_DIR}/$CONFIG_NAME" "$PROJECT_DIR/turbo.json"

# Check if there are changes before trying to run git commit
if [[ $(git status --porcelain) ]]; then
  git add .
  git commit --quiet -m "Use $CONFIG_NAME as turbo.json"
fi
