FROM node:18 AS base
    # Update apt-get and install the necessary libraries
    # This is mainly so that the `canvas` package can be installed
    RUN apt-get update && \
        apt-get install -y build-essential libcairo2-dev libpango1.0-dev libjpeg-dev libgif-dev librsvg2-dev

FROM base AS builder
    WORKDIR /app

    # This might be necessary when switching to alpine
    # Check https://github.com/nodejs/docker-node/tree/b4117f9333da4138b03a546ec926ef50a31506c3#nodealpine to understand why libc6-compat might be needed.
    # RUN apk add --no-cache libc6-compat

    RUN npm install -g turbo

    COPY . .

    RUN turbo prune --scope=api --docker

FROM base as installer
    WORKDIR /app

    RUN npm install -g pnpm
    RUN npm install -g turbo

    # First install dependencies (as they change less often)
    COPY .gitignore .gitignore
    COPY --from=builder /app/out/json/ .
    COPY --from=builder /app/out/pnpm-lock.yaml ./pnpm-lock.yaml
    RUN pnpm install

    # Build the project and its dependencies
    COPY --from=builder /app/out/full/ .
    COPY turbo.json turbo.json

    # Uncomment and use build args to enable remote caching
    # ARG TURBO_TEAM
    # ENV TURBO_TEAM=$TURBO_TEAM

    # ARG TURBO_TOKEN
    # ENV TURBO_TOKEN=$TURBO_TOKEN

    RUN turbo run build --filter=web...

FROM base AS runner
  WORKDIR /app

  RUN npm install -g pnpm

  # Don't run production as root
  RUN addgroup --system --gid 1001 nodejs
  RUN adduser --system --uid 1001 nextjs
  USER nextjs

  COPY --from=installer /app/apps/web/next.config.js .
  COPY --from=installer /app/apps/web/package.json .

  # Automatically leverage output traces to reduce image size
  # https://nextjs.org/docs/advanced-features/output-file-tracing
  COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/standalone ./
  COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/static ./apps/web/.next/static
  COPY --from=installer --chown=nextjs:nodejs /app/apps/web/public ./apps/web/public

  CMD pnpm --filter web run start
