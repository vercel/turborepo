#!/bin/bash

# This script is run before Claude Code evaluation starts
# It receives these environment variables:
#   $PORT - The port the dev server is running on
#   $OUTPUT_DIR - Path to the output directory where Claude is working
#   $EVAL_NAME - Name of the current eval (e.g., "001-server-component")
#   $EVAL_DIR - Path to the eval directory

echo "🔧 Setting up MCP server for $EVAL_NAME"
echo "   Dev server running on port $PORT"
echo "   Output directory: $OUTPUT_DIR"

# Write .mcp.json to the output directory for Claude Code to discover
cat > "$OUTPUT_DIR/.mcp.json" <<EOF
{
  "mcpServers": {
    "next-devtools": {
      "command": "npx",
      "args": ["-y", "next-devtools-mcp@latest"]
    }
  }
}
EOF

echo "✅ MCP server configured in .mcp.json"
