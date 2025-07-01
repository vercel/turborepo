#!/bin/bash

# Convex Post-Create Setup Script
# This script should only be run ONCE after the devcontainer or Codespace is created.
echo "🚀 Running post-create setup for Convex..."

echo "📦 Installing project dependencies..."
pnpm config set ignore-scripts false
pnpm install

# Checking and creating .env files
env_file=".env.local"
example_env_file=".env.local.example"
docker_env_file=".env.docker"
docker_example_env_file=".env.docker.example"
if [ ! -f "$env_file" ]; then
    if [ -f "$example_env_file" ]; then
        cp "$example_env_file" "$env_file"
        echo "Created $env_file from $example_env_file."
    else
        echo "❌ $env_file does not exist and $example_env_file not found."
        exit 1
    fi
fi

if [ ! -f "$docker_env_file" ]; then
    if [ -f "$docker_example_env_file" ]; then
        cp "$docker_example_env_file" "$docker_env_file"
        echo "Created $docker_env_file from $docker_example_env_file."
    else
        echo "❌ $docker_env_file does not exist and $docker_example_env_file not found."
        exit 1
    fi
fi

# Set CONVEX_SELF_HOSTED_URL in .env.local if running in Codespaces
if [ -n "$CODESPACE_NAME" ]; then
    CONVEX_SELF_HOSTED_URL="https://${CODESPACE_NAME}-3210.app.github.dev"
    if grep -q '^CONVEX_SELF_HOSTED_URL=' "$env_file"; then
        sed -i "s#^CONVEX_SELF_HOSTED_URL=.*#CONVEX_SELF_HOSTED_URL=$CONVEX_SELF_HOSTED_URL#" "$env_file"
    else
        echo "CONVEX_SELF_HOSTED_URL=$CONVEX_SELF_HOSTED_URL" >> "$env_file"
    fi
    echo "Set CONVEX_SELF_HOSTED_URL to $CONVEX_SELF_HOSTED_URL in $env_file."

    # Update Convex URLs in .env.docker for Codespaces
    CONVEX_CLOUD_ORIGIN="https://${CODESPACE_NAME}-3210.app.github.dev"
    CONVEX_SITE_ORIGIN="https://${CODESPACE_NAME}-3211.app.github.dev"
    NEXT_PUBLIC_DEPLOYMENT_URL="https://${CODESPACE_NAME}-3210.app.github.dev"
    sed -i "s#^CONVEX_CLOUD_ORIGIN=.*#CONVEX_CLOUD_ORIGIN=$CONVEX_CLOUD_ORIGIN#" "$docker_env_file"
    sed -i "s#^CONVEX_SITE_ORIGIN=.*#CONVEX_SITE_ORIGIN=$CONVEX_SITE_ORIGIN#" "$docker_env_file"
    sed -i "s#^NEXT_PUBLIC_DEPLOYMENT_URL=.*#NEXT_PUBLIC_DEPLOYMENT_URL=$NEXT_PUBLIC_DEPLOYMENT_URL#" "$docker_env_file"
    echo "Set Convex URLs for Codespaces in $docker_env_file."
fi

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Docker is not running. Please start Docker and try again."
    exit 1
fi

echo "🐳 Starting Docker containers..."
pnpm run docker:up

# Set Codespace ports 3210 and 5173 to public
if [ -n "$CODESPACE_NAME" ]; then
    gh codespace ports visibility 3210:public --codespace "$CODESPACE_NAME"
    echo "Set Codespace port 3210 to public."
    gh codespace ports visibility 5173:public --codespace "$CODESPACE_NAME"
    echo "Set Codespace port 5173 to public."
fi

echo "🔑 Generating admin key (for Docker backend)..."
ADMIN_KEY=$(docker compose exec -T backend ./generate_admin_key.sh)
if [ -z "$ADMIN_KEY" ]; then
    echo "❌ Failed to generate admin key. Check logs with: npm run docker:logs"
    exit 1
fi

if grep -q '^CONVEX_SELF_HOSTED_ADMIN_KEY=' "$env_file"; then
    sed -i "s#^CONVEX_SELF_HOSTED_ADMIN_KEY=.*#CONVEX_SELF_HOSTED_ADMIN_KEY=$ADMIN_KEY#" "$env_file"
else
    echo "CONVEX_SELF_HOSTED_ADMIN_KEY=$ADMIN_KEY" >> "$env_file"
fi

echo "🚀 Deploying Convex functions..."
pnpm run deploy-functions

echo "🐳 Stopping Docker containers..."
pnpm run docker:down
