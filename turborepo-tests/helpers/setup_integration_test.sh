#!/usr/bin/env bash

set -eo pipefail

INSTALL_DEPS=true
ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-install)
      INSTALL_DEPS=false
      shift # past the option
      ;;
    *)
      ARGS+=("$1")
      shift
  esac
done


FIXTURE_NAME="${ARGS[0]-basic_monorepo}"

# Default to version of npm installed with Node 18.20.2
# If CI is failing, check that this version is the same as
# the CI runner's version of npm
PACKAGE_MANAGER="npm@10.5.0"
if [[ "${ARGS[1]}" != "" ]]; then
  PACKAGE_MANAGER="${ARGS[1]}"
fi

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"

TARGET_DIR="$(pwd)"

# on macos, using the tmp dir set by prysk can fail, so set it
# to /tmp which is less secure (777) but wont crash
if [[ "$OSTYPE" == darwin* ]]; then
  export TMPDIR=/tmp
fi

# Shared fixture cache to avoid redundant npm installs
# Cache key includes both fixture name and package manager version
# Use TMPDIR for cross-platform compatibility (works on macOS, Linux, Windows/MSYS)
CACHE_BASE_DIR="${TMPDIR:-/tmp}/turbo-fixture-cache"
CACHE_KEY="${FIXTURE_NAME}-${PACKAGE_MANAGER//\//_}" # Replace / with _ for filesystem safety
FIXTURE_CACHE="${CACHE_BASE_DIR}/${CACHE_KEY}"

if $INSTALL_DEPS; then
  # Check if cache is ready (both directory and .ready marker exist)
  if [ ! -f "${FIXTURE_CACHE}/.ready" ]; then
    # Try to atomically create the cache directory (handles parallel test races)
    if mkdir "$FIXTURE_CACHE" 2>/dev/null; then
      # We won the race - build the cache
      # Copy fixture to cache location
      "${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${FIXTURE_CACHE}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/fixtures"

      # Setup git in cache (needed for install_deps.sh to commit)
      "${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${FIXTURE_CACHE}"

      # Setup package manager and install deps in cache
      # Use subshell to avoid changing caller's working directory
      "${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${FIXTURE_CACHE}" "$PACKAGE_MANAGER"
      (cd "${FIXTURE_CACHE}" && "${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER")

      # Remove .git from cache since each test needs its own git repo
      rm -rf "${FIXTURE_CACHE}/.git"

      # Mark cache as ready - this must be the LAST step
      touch "${FIXTURE_CACHE}/.ready"
    else
      # Someone else is building the cache - wait for .ready marker
      # Timeout after 60 seconds to prevent infinite waits
      WAIT_COUNT=0
      MAX_WAIT=600 # 60 seconds (600 * 0.1s)
      while [ ! -f "${FIXTURE_CACHE}/.ready" ] && [ $WAIT_COUNT -lt $MAX_WAIT ]; do
        sleep 0.1
        WAIT_COUNT=$((WAIT_COUNT + 1))
      done

      # If timeout, assume first builder failed - clean up and retry
      if [ ! -f "${FIXTURE_CACHE}/.ready" ]; then
        rm -rf "${FIXTURE_CACHE}"
        # Retry cache creation ourselves
        if mkdir "$FIXTURE_CACHE" 2>/dev/null; then
          "${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${FIXTURE_CACHE}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/fixtures"
          "${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${FIXTURE_CACHE}"
          "${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${FIXTURE_CACHE}" "$PACKAGE_MANAGER"
          (cd "${FIXTURE_CACHE}" && "${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER")
          rm -rf "${FIXTURE_CACHE}/.git"
          touch "${FIXTURE_CACHE}/.ready"
        fi
      fi
    fi
  fi

  # Use cached fixture with pre-installed dependencies
  cp -a "${FIXTURE_CACHE}/." "${TARGET_DIR}/"

  # Setup fresh git repo for this test
  "${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"

  # Commit the already-installed dependencies
  # Use subshell to avoid changing caller's working directory
  (cd "${TARGET_DIR}" && git add . && if [[ $(git status --porcelain) ]]; then git commit -am "Install dependencies" --quiet > /dev/null 2>&1 || true; fi)
else
  # No caching: use original flow
  "${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/fixtures"
  "${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"
  "${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${TARGET_DIR}" "$PACKAGE_MANAGER"
  if $INSTALL_DEPS; then
    "${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER"
  fi
fi

# Set TURBO env var, it is used by tests to run the binary
if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

export TURBO_TELEMETRY_MESSAGE_DISABLED=1
export TURBO_GLOBAL_WARNING_DISABLED=1
export TURBO_PRINT_VERSION_DISABLED=1
export TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}

# Undo the set -eo pipefail at the top of this script
# This script is called with a leading ".", which means that it does not run
# in a new child process, so the set -eo pipefail would affect the calling script.
# Some of our tests actually assert non-zero exit codes, and we don't want to
# abort the test in those cases. So we undo the set -eo pipefail here.
set +eo pipefail
