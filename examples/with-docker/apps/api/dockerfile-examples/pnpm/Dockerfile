# ───────────────────────────────────────────────────────
# PNPM-based build
# ───────────────────────────────────────────────────────
FROM node:18-alpine AS base

# The web Dockerfile is copy-pasted into our main docs at /docs/handbook/deploying-with-docker.
# Make sure you update this Dockerfile, the Dockerfile in the web workspace and copy that over to Dockerfile in the docs.

FROM base AS builder
RUN apk update && apk add --no-cache libc6-compat
WORKDIR /app

# Set PNPM_HOME to a directory that exists
ENV PNPM_HOME=/pnpm
RUN mkdir -p /pnpm && \
    echo "export PATH=$PNPM_HOME:$PATH" >> /etc/profile

# Install pnpm and turbo globally
RUN corepack enable && corepack prepare pnpm@latest --activate
RUN pnpm config set global-bin-dir $PNPM_HOME
RUN pnpm add -g turbo

COPY . .
RUN turbo prune api --docker

# Add lockfile and package.json's of isolated subworkspace
FROM base AS installer
RUN apk update
RUN apk add --no-cache libc6-compat
WORKDIR /app

# First install dependencies (as they change less often)
COPY --from=builder /app/out/json/ .
RUN pnpm install

# Build the project and its dependencies
COPY --from=builder /app/out/full/ .

# Uncomment and use build args to enable remote caching
# ARG TURBO_TEAM
# ENV TURBO_TEAM=$TURBO_TEAM

# ARG TURBO_TOKEN
# ENV TURBO_TOKEN=$TURBO_TOKEN

RUN pnpm turbo build

FROM base AS runner
WORKDIR /app

# Don't run production as root
RUN addgroup --system --gid 1001 expressjs
RUN adduser --system --uid 1001 expressjs
USER expressjs
COPY --from=installer /app .

CMD node apps/api/dist/index.js
