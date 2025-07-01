# 🏃‍♂️ Local Development Setup Guide

This guide covers setting up and running the Telegram Bot + Convex Backend + Turborepo project locally without Docker.

## 📋 Prerequisites

- **Node.js 18+** - [Download here](https://nodejs.org/)
- **pnpm** - [Installation guide](https://pnpm.io/installation)
- **Go 1.21+** - [Download here](https://golang.org/dl/)
- **Git** - [Download here](https://git-scm.com/downloads)

## 🚀 Initial Setup

### 1. Clone and Install Dependencies

```bash
# Clone the repository
git clone https://github.com/kessenma/go-convex-telegram-turborepo
cd go-convex-telegram-turborepo

# Install all dependencies across the monorepo
pnpm install
```

### 2. Environment Configuration

```bash
# Copy environment template
cp .env.example .env

# Edit .env with your configuration
# Required variables:
# - TELEGRAM_TOKEN (from @BotFather)
# - CONVEX_URL (for self-hosted Convex)
# - NEXT_PUBLIC_CONVEX_URL (for web app)
```

## 🏗️ Service Setup

### Convex Backend (Self-Hosted)

```bash
# Navigate to Convex directory
cd apps/docker-convex

# Start self-hosted Convex backend
pnpm run dev

# This will:
# 1. Start Docker containers for Convex
# 2. Generate admin keys
# 3. Deploy functions
```

### Next.js Web Application

```bash
# In a new terminal, navigate to web app
cd apps/web

# Start development server
pnpm run dev

# Access at: http://localhost:3000
```

### Go Telegram Bot

```bash
# In a new terminal, navigate to bot directory
cd apps/golang-telegram-bot

# Install Go dependencies
make install

# Set up environment (if needed)
make env-setup

# Start development server
make dev
```

## ⚡ Turborepo Commands

### Development Commands

```bash
# Run all services in development mode
pnpm run dev

# Run individual services
pnpm run dev:web      # Next.js web app
pnpm run dev:convex   # Convex backend
pnpm run dev:bot      # Go Telegram bot
```

### Build Commands

```bash
# Build all applications
pnpm run build

# Build individual applications
pnpm run build:web    # Next.js production build
pnpm run build:convex # Deploy Convex functions
pnpm run build:bot    # Compile Go binary
```

### Production Commands

```bash
# Start all services in production mode
pnpm run start

# Start individual services
pnpm run start:web    # Next.js production server
pnpm run start:convex # Convex production setup
pnpm run start:bot    # Run compiled Go binary
```

### Maintenance Commands

```bash
# Clean build artifacts
pnpm run clean        # Clean all
pnpm run clean:web    # Clean Next.js build
pnpm run clean:convex # Clean Convex containers
pnpm run clean:bot    # Clean Go build artifacts

# Lint code
pnpm run lint         # Lint all projects

# Type checking
pnpm run check-types  # TypeScript type checking
```

## 🔧 Turborepo Cache Management

### Understanding the Cache

Turborepo intelligently caches build outputs and task results:

```bash
# View cache status (dry run)
pnpm turbo run build --dry-run

# Force rebuild (bypass cache)
pnpm turbo run build --force

# Clear all caches
pnpm turbo run clean
```

### Cache Benefits

- **⚡ Instant rebuilds** when nothing changed
- **🎯 Selective rebuilds** only for modified packages
- **🔄 Dependency awareness** rebuilds dependents automatically
- **📊 Task parallelization** runs independent tasks simultaneously

### Cache Invalidation

Cache automatically invalidates when:

- Source files change
- Dependencies update
- Environment variables change
- Configuration files modify

## 🛠️ Development Workflow

### Typical Development Session

1. **Start all services:**

   ```bash
   pnpm run dev
   ```

2. **Make changes to any app**

3. **Test changes:**

   ```bash
   # Lint your changes
   pnpm run lint

   # Type check
   pnpm run check-types

   # Build to verify
   pnpm run build
   ```

4. **Clean up when done:**
   ```bash
   pnpm run clean
   ```

### Working on Individual Apps

**Web App Development:**

```bash
cd apps/web
pnpm run dev
# Make changes, hot reload active
```

**Bot Development:**

```bash
cd apps/golang-telegram-bot
make dev
# Make changes, restart with make dev
```

**Convex Functions:**

```bash
cd apps/docker-convex
# Edit functions in convex/ directory
pnpm run deploy-functions
```

## 🐛 Troubleshooting

### Common Issues

**Port conflicts:**

```bash
# Check what's running on ports
lsof -i :3000  # Next.js
lsof -i :3210  # Convex
lsof -i :8080  # Bot (if applicable)
```

**Cache issues:**

```bash
# Clear all caches and rebuild
pnpm run clean
pnpm run build --force
```

**Dependency issues:**

```bash
# Reinstall all dependencies
rm -rf node_modules apps/*/node_modules
pnpm install:all
```

**Environment issues:**

```bash
# Verify environment variables
cat .env
# Ensure all required variables are set
```

### Getting Help

- Check individual app READMEs in `apps/*/README.md`
- Review Docker setup in main README for comparison
- Check Turborepo docs: https://turbo.build/repo/docs

## 📚 Additional Resources

- [Turborepo Documentation](https://turbo.build/repo/docs)
- [Next.js Documentation](https://nextjs.org/docs)
- [Convex Documentation](https://docs.convex.dev/)
- [Go Documentation](https://golang.org/doc/)
- [pnpm Documentation](https://pnpm.io/)
