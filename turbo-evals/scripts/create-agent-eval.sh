#!/bin/bash
# Usage: ./scripts/create-agent-eval.sh <agent-number> <original-eval-name> "<prompt>" "<readme-description>"

set -e

AGENT_NUM="$1"
ORIGINAL_EVAL="$2"
PROMPT="$3"
README_DESC="$4"

AGENT_DIR="evals/agent-$AGENT_NUM-$ORIGINAL_EVAL"

echo "Creating $AGENT_DIR..."

# Create directory structure
mkdir -p "$AGENT_DIR/input"

# Copy base Next.js app structure from agent-001
cp -r evals/agent-001-add-dark-mode-toggle/input/* "$AGENT_DIR/input/"

# Create prompt.md
cat > "$AGENT_DIR/prompt.md" <<EOF
A Next.js app is running at http://localhost:3000

$PROMPT
EOF

# Create README.md
cat > "$AGENT_DIR/README.md" <<EOF
# Agent Eval: $ORIGINAL_EVAL

$README_DESC

## Expected behavior
Check the prompt.md for specific requirements.
EOF

echo "✅ Created $AGENT_DIR"
