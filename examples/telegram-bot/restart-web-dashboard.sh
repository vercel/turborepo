#!/bin/bash

echo "🔄 Rebuilding and restarting web dashboard with correct Convex configuration..."

# Stop the web dashboard service
echo "⏹️  Stopping web-dashboard service..."
docker-compose stop web-dashboard

# Remove the existing web dashboard container and image to force rebuild
echo "🗑️  Removing existing web-dashboard container and image..."
docker-compose rm -f web-dashboard
docker rmi telegram-bot-web-dashboard 2>/dev/null || true

# Rebuild and start the web dashboard
echo "🔨 Rebuilding web-dashboard service..."
docker-compose build --no-cache web-dashboard

echo "🚀 Starting web-dashboard service..."
docker-compose up -d web-dashboard

echo "✅ Web dashboard restarted! Check the logs with:"
echo "   docker-compose logs -f web-dashboard"
echo ""
echo "🌐 Your web dashboard should be available at: http://localhost:3000"
echo "🔧 Convex backend is available at: http://localhost:3211"
echo "📊 Convex dashboard is available at: http://localhost:6791"
