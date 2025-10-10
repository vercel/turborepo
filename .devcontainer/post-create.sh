#!/bin/bash
set -e

echo "🚀 Setting up Turborepo development environment..."

# Ensure we're in the workspace directory
cd /workspaces/turborepo 2>/dev/null || cd /workspace 2>/dev/null || cd "$(pwd)"

echo "📦 Installing Node.js dependencies..."
# Install dependencies using pnpm
pnpm install

echo "🦀 Verifying Rust installation..."
# Verify Rust toolchain matches rust-toolchain.toml
rustc --version
cargo --version

# Install dependencies and build the project to verify everything works
echo "🔨 Building Turborepo..."
cargo build

echo "✅ Turborepo development environment is ready!"
echo ""
echo "📋 Quick start commands:"
echo "  cargo build          - Build the Rust binary"
echo "  cargo test           - Run unit tests"
echo "  cargo coverage       - Run tests with coverage"
echo "  pnpm test            - Run integration tests" 
echo "  pnpm build           - Build all packages"
echo ""
echo "🔍 For more commands, see CONTRIBUTING.md"