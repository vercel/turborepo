#!/bin/bash

THIS_SCRIPT=$(dirname "${BASH_SOURCE[0]}")
TURBOREPO_TESTS_DIR="$THIS_SCRIPT/../../.."

# this env var will be used in sourced scripts
FIXTURE_NAME="${1-basic_monorepo}"

# Run global setup script. Using source means that it executes the script in the current
# shell instead of a subshell, so env vars are preserved.
source "${TURBOREPO_TESTS_DIR}/helpers/setup.sh"
source "${TURBOREPO_TESTS_DIR}/helpers/setup_repo.sh"
