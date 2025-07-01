# Makefile for the Go Telegram Bot - Package.json-like interface
# Comprehensive development workflow automation

# Use .PHONY to declare targets that are not actual files
.PHONY: help install dev build run start stop restart clean clean-all logs shell test lint format deps update status env-check env-setup docker-prune git-setup rebuild fresh-start dev-cycle full-reset

# ---- Variables ----
APP_NAME=my-bot
CONTAINER_NAME=$(APP_NAME)-container
GO_VERSION=1.24
DOCKER_REGISTRY=localhost
TAG=latest

# Colors for output
RED=\033[0;31m
GREEN=\033[0;32m
YELLOW=\033[1;33m
BLUE=\033[0;34m
NC=\033[0m # No Color

# ---- Main Commands ----

# Default help command
help:
	@echo "$(BLUE)📦 Go Telegram Bot - Development Interface$(NC)"
	@echo ""
	@echo "$(GREEN)🚀 Quick Start:$(NC)"
	@echo "  make install       Install dependencies and setup environment"
	@echo "  make dev           Run in development mode (local)"
	@echo "  make start         Build and run in Docker"
	@echo ""
	@echo "$(GREEN)🔧 Development:$(NC)"
	@echo "  make build         Build Docker image"
	@echo "  make run           Run container with environment"
	@echo "  make stop          Stop running container"
	@echo "  make restart       Restart the application"
	@echo "  make logs          View container logs"
	@echo "  make shell         Open shell in container"
	@echo ""
	@echo "$(GREEN)🧹 Cleanup:$(NC)"
	@echo "  make clean         Stop and remove app containers"
	@echo "  make clean-all     Remove all containers and images"
	@echo "  make docker-prune  Clean up Docker system"
	@echo ""
	@echo "$(GREEN)🔍 Quality & Testing:$(NC)"
	@echo "  make test          Run tests"
	@echo "  make lint          Run linter"
	@echo "  make format        Format code"
	@echo "  make deps          Update dependencies"
	@echo ""
	@echo "$(GREEN)⚙️  Utilities:$(NC)"
	@echo "  make status        Show system status"
	@echo "  make env-check     Verify environment setup"
	@echo "  make env-setup     Setup environment files"
	@echo "  make git-setup     Setup git with main branch"
	@echo ""
	@echo "$(GREEN)🔗 Composite Commands:$(NC)"
	@echo "  make rebuild       Clean, build, and restart"
	@echo "  make fresh-start   Complete cleanup and fresh start"
	@echo "  make dev-cycle     Format, test, and restart"
	@echo "  make full-reset    Nuclear option: clean everything and rebuild"

# ---- Installation & Setup ----

install: env-check
	@echo "$(GREEN)📦 Installing dependencies...$(NC)"
	go mod download
	go mod tidy
	@echo "$(GREEN)✅ Dependencies installed$(NC)"

env-setup:
	@echo "$(YELLOW)🔧 Setting up environment...$(NC)"
	@if [ ! -f .env ]; then \
		if [ -f .env.example ]; then \
			cp .env.example .env; \
			echo "$(GREEN)✅ Created .env from .env.example$(NC)"; \
			echo "$(YELLOW)⚠️  Please edit .env with your actual values$(NC)"; \
		else \
			echo "TELEGRAM_TOKEN=your_bot_token_here" > .env; \
			echo "$(GREEN)✅ Created basic .env file$(NC)"; \
			echo "$(YELLOW)⚠️  Please edit .env with your actual token$(NC)"; \
		fi; \
	else \
		echo "$(GREEN)✅ .env file already exists$(NC)"; \
	fi

env-check:
	@echo "$(BLUE)🔍 Checking environment...$(NC)"
	@if [ ! -f .env ]; then \
		echo "$(RED)❌ .env file not found$(NC)"; \
		echo "$(YELLOW)Run 'make env-setup' to create it$(NC)"; \
		exit 1; \
	fi
	@if ! grep -q "TELEGRAM_TOKEN=" .env || grep -q "your_bot_token_here" .env; then \
		echo "$(YELLOW)⚠️  Please set your TELEGRAM_TOKEN in .env$(NC)"; \
	fi
	@echo "$(GREEN)✅ Environment check complete$(NC)"

git-setup:
	@echo "$(BLUE)🔧 Setting up git with main branch...$(NC)"
	@if git rev-parse --git-dir > /dev/null 2>&1; then \
		if git show-ref --verify --quiet refs/heads/master; then \
			git branch -m master main; \
			echo "$(GREEN)✅ Renamed master branch to main$(NC)"; \
		else \
			echo "$(GREEN)✅ Already using main branch or no master branch found$(NC)"; \
		fi; \
	else \
		echo "$(YELLOW)⚠️  Not a git repository$(NC)"; \
	fi

# ---- Development Commands ----

dev: env-check
	@echo "$(GREEN)🚀 Starting development mode (local)...$(NC)"
	go run main.go

build:
	@echo "$(GREEN)🔨 Building Docker image: $(APP_NAME):$(TAG)...$(NC)"
	docker build -t $(APP_NAME):$(TAG) .
	@echo "$(GREEN)✅ Build complete$(NC)"

run: env-check build
	@echo "$(GREEN)🚀 Running $(APP_NAME) in Docker...$(NC)"
	@docker stop $(CONTAINER_NAME) 2>/dev/null || true
	@docker rm $(CONTAINER_NAME) 2>/dev/null || true
	docker run -d --name $(CONTAINER_NAME) --env-file .env $(APP_NAME):$(TAG)
	@echo "$(GREEN)✅ Container started: $(CONTAINER_NAME)$(NC)"

start: run
	@echo "$(GREEN)✅ Application started successfully$(NC)"

stop:
	@echo "$(YELLOW)🛑 Stopping $(CONTAINER_NAME)...$(NC)"
	@docker stop $(CONTAINER_NAME) 2>/dev/null || echo "$(YELLOW)Container not running$(NC)"
	@echo "$(GREEN)✅ Container stopped$(NC)"

restart: stop start
	@echo "$(GREEN)🔄 Application restarted$(NC)"

logs:
	@echo "$(BLUE)📋 Showing logs for $(CONTAINER_NAME)...$(NC)"
	@docker logs -f $(CONTAINER_NAME) 2>/dev/null || echo "$(RED)❌ Container not found or not running$(NC)"

shell:
	@echo "$(BLUE)🐚 Opening shell in $(CONTAINER_NAME)...$(NC)"
	@docker exec -it $(CONTAINER_NAME) /bin/sh 2>/dev/null || echo "$(RED)❌ Container not running$(NC)"

# ---- Testing & Quality ----

test:
	@echo "$(BLUE)🧪 Running tests...$(NC)"
	go test -v ./...
	@echo "$(GREEN)✅ Tests complete$(NC)"

lint:
	@echo "$(BLUE)🔍 Running linter...$(NC)"
	@if command -v golangci-lint >/dev/null 2>&1; then \
		golangci-lint run; \
	else \
		echo "$(YELLOW)⚠️  golangci-lint not installed, running go vet instead$(NC)"; \
		go vet ./...; \
	fi
	@echo "$(GREEN)✅ Linting complete$(NC)"

format:
	@echo "$(BLUE)✨ Formatting code...$(NC)"
	go fmt ./...
	go mod tidy
	@echo "$(GREEN)✅ Code formatted$(NC)"

deps:
	@echo "$(BLUE)📦 Updating dependencies...$(NC)"
	go get -u ./...
	go mod tidy
	@echo "$(GREEN)✅ Dependencies updated$(NC)"

# ---- Cleanup Commands ----

clean:
	@echo "$(YELLOW)🧹 Cleaning up app containers...$(NC)"
	@docker stop $(CONTAINER_NAME) 2>/dev/null || true
	@docker rm $(CONTAINER_NAME) 2>/dev/null || true
	@echo "$(GREEN)✅ App containers cleaned$(NC)"

clean-all: clean
	@echo "$(YELLOW)🧹 Removing all containers and images...$(NC)"
	@docker stop $$(docker ps -a -q) 2>/dev/null || true
	@docker rm $$(docker ps -a -q) 2>/dev/null || true
	@docker rmi $(APP_NAME):$(TAG) 2>/dev/null || true
	@echo "$(GREEN)✅ All containers and images removed$(NC)"

docker-prune:
	@echo "$(YELLOW)🧹 Pruning Docker system...$(NC)"
	docker system prune -f
	docker volume prune -f
	@echo "$(GREEN)✅ Docker system cleaned$(NC)"

# ---- Status & Information ----

status:
	@echo "$(BLUE)📊 System Status$(NC)"
	@echo ""
	@echo "$(GREEN)🐳 Docker:$(NC)"
	@docker --version 2>/dev/null || echo "$(RED)❌ Docker not installed$(NC)"
	@echo ""
	@echo "$(GREEN)🐹 Go:$(NC)"
	@go version 2>/dev/null || echo "$(RED)❌ Go not installed$(NC)"
	@echo ""
	@echo "$(GREEN)📦 Application:$(NC)"
	@if docker ps --format "table {{.Names}}\t{{.Status}}" | grep -q $(CONTAINER_NAME); then \
		echo "$(GREEN)✅ Container running$(NC)"; \
		docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" | grep $(CONTAINER_NAME); \
	else \
		echo "$(YELLOW)⚠️  Container not running$(NC)"; \
	fi
	@echo ""
	@echo "$(GREEN)🔧 Environment:$(NC)"
	@if [ -f .env ]; then \
		echo "$(GREEN)✅ .env file exists$(NC)"; \
	else \
		echo "$(RED)❌ .env file missing$(NC)"; \
	fi

# ---- Composite Commands ----

# Complete rebuild: clean, build, and restart
rebuild: clean build start
	@echo "$(GREEN)🔄 Complete rebuild finished$(NC)"

# Fresh start: clean everything and start fresh
fresh-start: clean-all docker-prune start
	@echo "$(GREEN)✨ Fresh start complete$(NC)"

# Development cycle: format, test, and restart
dev-cycle: format test restart
	@echo "$(GREEN)🔄 Development cycle complete$(NC)"

# Nuclear option: clean everything and rebuild from scratch
full-reset: stop clean-all docker-prune build start
	@echo "$(GREEN)💥 Full reset complete - everything rebuilt from scratch$(NC)"

