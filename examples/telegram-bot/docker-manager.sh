#!/bin/bash

# Docker Service Manager for Telegram Bot + Convex Backend
# Interactive script for managing individual Docker services

set -e

echo "🐳 Docker Service Manager"
echo "========================"
echo "Select a service to manage:"
echo ""
echo "📦 REBUILD (Stop → Remove → Build → Start):"
echo "  1. Rebuild Go Telegram Bot"
echo "  2. Rebuild Web Dashboard"
echo "  3. Rebuild Convex Backend + Dashboard"
echo "  4. Rebuild All Services"
echo ""
echo "🔄 RESTART (Quick restart without rebuild):"
echo "  5. Restart Go Telegram Bot"
echo "  6. Restart Web Dashboard"
echo "  7. Restart Convex Backend + Dashboard"
echo "  8. Restart All Services"
echo ""
echo "📋 LOGS (Real-time log viewing):"
echo "  9. View Bot Logs"
echo "  10. View Web Dashboard Logs"
echo "  11. View Convex Backend Logs"
echo "  12. View All Service Logs"
echo ""
echo "⚡ QUICK ACTIONS:"
echo "  13. Stop All Services"
echo "  14. Start All Services"
echo "  15. Check Service Status"
echo "  16. Test API Health"
echo "  17. Generate Admin Key"
echo "  18. Refresh Convex API"
echo ""
echo "  0. Exit"
echo ""
read -p "Enter your choice (0-18): " choice

case $choice in
    1)
        echo "🔄 Rebuilding Go Telegram Bot..."
        echo "Stopping telegram-bot..."
        docker compose stop telegram-bot
        echo "Removing telegram-bot container..."
        docker compose rm -f telegram-bot
        echo "Building and starting telegram-bot..."
        docker compose up --build -d telegram-bot
        echo "✅ Go Telegram Bot rebuilt successfully!"
        echo "📋 View logs with: docker compose logs -f telegram-bot"
        ;;
    2)
        echo "🔄 Rebuilding Web Dashboard..."
        echo "Stopping web-dashboard..."
        docker compose stop web-dashboard
        echo "Removing web-dashboard container..."
        docker compose rm -f web-dashboard
        echo "Building and starting web-dashboard..."
        docker compose up --build -d web-dashboard
        echo "✅ Web Dashboard rebuilt successfully!"
        echo "🌐 Access at: http://localhost:${WEB_DASHBOARD_PORT:-3000}"
        ;;
    3)
        echo "🔄 Rebuilding Convex Backend + Dashboard..."
        echo "Stopping convex services..."
        docker compose stop convex-backend convex-dashboard
        echo "Removing convex containers..."
        docker compose rm -f convex-backend convex-dashboard
        echo "Building and starting convex services..."
        docker compose up --build -d convex-backend convex-dashboard
        echo "✅ Convex services rebuilt successfully!"
        echo "🌐 Dashboard: http://localhost:6791"
        echo "🔍 API: http://localhost:3211"
        ;;
    4)
        echo "🔄 Rebuilding All Services..."
        docker compose down
        docker compose up --build -d
        echo "✅ All services rebuilt successfully!"
        ;;
    5)
        echo "🔄 Restarting Go Telegram Bot..."
        docker compose restart telegram-bot
        echo "✅ Bot restarted!"
        ;;
    6)
        echo "🔄 Restarting Web Dashboard..."
        docker compose restart web-dashboard
        echo "✅ Web Dashboard restarted!"
        ;;
    7)
        echo "🔄 Restarting Convex Backend + Dashboard..."
        docker compose restart convex-backend convex-dashboard
        echo "✅ Convex services restarted!"
        ;;
    8)
        echo "🔄 Restarting All Services..."
        docker compose restart
        echo "✅ All services restarted!"
        ;;
    9)
        echo "📋 Viewing Bot Logs (Press Ctrl+C to exit)..."
        docker compose logs -f telegram-bot
        ;;
    10)
        echo "📋 Viewing Web Dashboard Logs (Press Ctrl+C to exit)..."
        docker compose logs -f web-dashboard
        ;;
    11)
        echo "📋 Viewing Convex Backend Logs (Press Ctrl+C to exit)..."
        docker compose logs -f convex-backend
        ;;
    12)
        echo "📋 Viewing All Service Logs (Press Ctrl+C to exit)..."
        docker compose logs -f
        ;;
    13)
        echo "⏹️ Stopping All Services..."
        docker compose stop
        echo "✅ All services stopped!"
        ;;
    14)
        echo "▶️ Starting All Services..."
        docker compose start
        echo "✅ All services started!"
        ;;
    15)
        echo "📊 Service Status:"
        docker compose ps
        ;;
    16)
        echo "🔍 Testing API Health..."
        echo "Backend Health:"
        curl -f http://localhost:3210/version 2>/dev/null && echo "✅ Backend OK" || echo "❌ Backend Down"
        echo "API Health:"
        curl -f http://localhost:3211/api/health 2>/dev/null && echo "✅ API OK" || echo "❌ API Down"
        echo "Messages Endpoint:"
        curl -f http://localhost:3211/api/telegram/messages 2>/dev/null && echo "✅ Messages API OK" || echo "❌ Messages API Down"
        ;;
    17)
        echo "🔑 Generating Admin Key..."
        pnpm get-admin-key
        echo "✅ Admin key generated successfully!"
        echo "📋 The admin key has been saved to the admin-key directory."
        ;;
    18)
        echo "🔄 Refreshing Convex API..."
        echo "Deploying Convex functions..."
        pnpm convex:deploy
        echo "✅ Convex API refreshed successfully!"
        echo "🔍 New endpoints are now available at: http://localhost:3211"
        ;;
    0)
        echo "👋 Goodbye!"
        exit 0
        ;;
    *)
        echo "❌ Invalid option. Please try again."
        echo "Run the script again and choose a number between 0-18."
        exit 1
        ;;
esac

echo ""
echo "🔄 Run './docker-manager.sh' again to perform another action."