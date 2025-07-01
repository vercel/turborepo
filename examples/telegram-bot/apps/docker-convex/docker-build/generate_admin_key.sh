#!/bin/bash

set -e

source ./read_credentials.sh

ADMIN_KEY=$(./generate_key "$INSTANCE_NAME" "$INSTANCE_SECRET")

# Print to console
echo "$ADMIN_KEY"

# Save to timestamped file in admin-key directory
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
ADMIN_KEY_DIR="/convex/admin-key"
mkdir -p "$ADMIN_KEY_DIR"
FILE_PATH="$ADMIN_KEY_DIR/admin_key_$TIMESTAMP.md"

cat > "$FILE_PATH" << EOF
# Convex Admin Key

Generated: $(date)
Instance Name: $INSTANCE_NAME

## Admin Key
\`\`\`
$ADMIN_KEY
\`\`\`

## Usage
Use this admin key to access the Convex dashboard and manage your self-hosted instance.
EOF

echo "Admin key saved to: $FILE_PATH"
