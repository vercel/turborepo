#!/bin/bash

# Convex Self-Hosted Setup Script
echo "🚀 Setting up Convex Self-Hosted..."

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Docker is not running. Please start Docker and try again."
    exit 1
fi

# Check if .env.docker exists
if [ ! -f ".env.docker" ]; then
    echo "❌ .env.docker file not found. Please create it from .env.docker.example"
    exit 1
fi

# Start Docker containers
echo "🐳 Starting Docker containers..."
pnpm run docker:up

# Wait for backend to be ready
echo "⏳ Waiting for backend to start..."
sleep 15

# Check if backend is healthy
if ! curl -f http://localhost:3210/version > /dev/null 2>&1; then
    echo "❌ Backend is not responding. Check logs with: npm run docker:logs"
    exit 1
fi

echo ""
echo "✅ Setup complete!"
echo ""
echo "👉 Now run 'pnpm dev' to start the development server."
