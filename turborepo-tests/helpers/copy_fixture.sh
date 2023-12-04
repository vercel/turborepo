#!/bin/bash
set -e

TARGET_DIR=$1
FIXTURE="${2-basic_monorepo}"

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
MONOREPO_ROOT_DIR="${SCRIPT_DIR}/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"
FIXTURES_DIR="${TURBOREPO_TESTS_DIR}/integration/fixtures"

cp -a "${FIXTURES_DIR}/$FIXTURE/." "${TARGET_DIR}/"
