# ───────────────────────────────────────────────────────
# PNPM-based build
# ───────────────────────────────────────────────────────

FROM node:18-alpine AS base
# Enable Corepack and pin pnpm to the version in packageManager
RUN corepack enable \
 && corepack prepare pnpm@10.10.0 --activate

# Builder stage
FROM base AS builder
RUN apk update && apk add --no-cache libc6-compat
WORKDIR /app

# Install Turbo CLI
RUN pnpm add -g turbo

COPY . .
# Prune only the web workspace for Docker
RUN turbo prune web --docker

# Installer stage
FROM base AS installer
RUN apk update && apk add --no-cache libc6-compat
WORKDIR /app

# Copy lockfiles and package.jsons
COPY --from=builder /app/out/json/ .
# Install dependencies
RUN pnpm install --frozen-lockfile

# Copy full workspace and build
COPY --from=builder /app/out/full/ .
RUN pnpm turbo build

# Runner stage
FROM base AS runner
WORKDIR /app

# Create non-root user
RUN addgroup --system --gid 1001 nodejs \
 && adduser  --system --uid 1001 nextjs
USER nextjs

# Copy only what's needed to run Next.js standalone
COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/standalone ./
COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/static    ./apps/web/.next/static
COPY --from=installer --chown=nextjs:nodejs /app/apps/web/public         ./apps/web/public

CMD ["node", "apps/web/server.js"]
