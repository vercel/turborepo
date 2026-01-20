#!/bin/bash

# This script is run after Claude Code evaluation completes
# It receives these environment variables:
#   $PORT - The port the dev server was running on
#   $OUTPUT_DIR - Path to the output directory where Claude worked
#   $EVAL_NAME - Name of the current eval (e.g., "001-server-component")
#   $EVAL_DIR - Path to the eval directory

echo "🧹 Post-eval cleanup for $EVAL_NAME"

# Leave .mcp.json in place for inspection
# It will be cleaned up when the output directory is removed

echo "✅ Cleanup complete (.mcp.json preserved for inspection)"
