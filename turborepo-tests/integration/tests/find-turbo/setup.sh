#!/bin/bash

export TURBO_DOWNLOAD_LOCAL_ENABLED=0
SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2

echo "=== Setup starting for fixture: $FIXTURE_DIR ==="
echo "OSTYPE: $OSTYPE"
echo "TARGET_DIR: $TARGET_DIR"

cp -a ${SCRIPT_DIR}/../../fixtures/find_turbo/$FIXTURE_DIR/. ${TARGET_DIR}/

# Verify what got copied
echo "Checking .pnpm contents after cp:"
ls -la ${TARGET_DIR}/node_modules/.pnpm/ 2>&1 || echo ".pnpm directory does not exist"
if [ -d "${TARGET_DIR}/node_modules/.pnpm/turbo@1.0.0" ]; then
  echo "turbo@1.0.0 directory exists"
  ls -la ${TARGET_DIR}/node_modules/.pnpm/turbo@1.0.0/node_modules/ 2>&1 || echo "node_modules subdirectory missing"
else
  echo "ERROR: turbo@1.0.0 directory was NOT copied!"
fi

# We need to symlink: turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
# where `turbo` is the symlink
# and `.pnpm/turbo@1.0.0/node_modules/turbo` is the path to symlink to
# Note: using a nested if so it's easy to find the Windows checks in scripts around the codebase.
if [[ "$OSTYPE" == "msys" ]]; then
   echo "Running on Windows (msys)"
   if [[ $FIXTURE_DIR == "linked" ]]; then
    echo "Setting up linked fixture for Windows..."

    # Check what exists before we start
    echo "Before setup:"
    ls -la node_modules/turbo 2>&1 || echo "node_modules/turbo does not exist"
    ls -la node_modules/.pnpm/turbo@1.0.0/node_modules/turbo 2>&1 || echo "pnpm turbo directory does not exist"

    # Delete the existing turbo directory or file, whatever exists there
    echo "Removing existing node_modules/turbo..."
    rm -rf node_modules/turbo

    # Let's enter the node_modules directory
    echo "Entering node_modules directory..."
    pushd node_modules > /dev/null || exit 1

    # Use pnpx to run symlnk-dir because installing globally doesn't work with pnpm.
    echo "Attempting to create symlink with: pnpx symlink-dir .pnpm/turbo@1.0.0/node_modules/turbo turbo"
    if pnpx symlink-dir .pnpm/turbo@1.0.0/node_modules/turbo turbo; then
      echo "✓ Symlink created successfully"
    else
      EXIT_CODE=$?
      echo "✗ Symlink creation FAILED with exit code: $EXIT_CODE"
    fi

    # Get outta there
    popd > /dev/null || exit 1

    # Verify what we ended up with
    echo "After setup:"
    ls -la node_modules/turbo 2>&1 || echo "node_modules/turbo still does not exist"
    if [ -L node_modules/turbo ]; then
      LINK_TARGET=$(readlink node_modules/turbo)
      echo "node_modules/turbo is a symlink pointing to: $LINK_TARGET"
      echo "Checking if symlink target exists:"
      ls -la "node_modules/$LINK_TARGET" 2>&1 || echo "ERROR: Symlink target does NOT exist!"
      echo "Checking if we can find turbo-windows-64 from the symlink target:"
      ls -la "node_modules/$LINK_TARGET/../turbo-windows-64" 2>&1 || echo "ERROR: turbo-windows-64 not found relative to symlink target"
    fi

    echo "=== Setup complete ==="
  fi
else
  echo "Not running on Windows, skipping Windows-specific setup"
fi
