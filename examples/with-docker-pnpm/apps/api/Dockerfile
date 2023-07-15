FROM node:18-alpine AS base
    # Check https://github.com/nodejs/docker-node/tree/b4117f9333da4138b03a546ec926ef50a31506c3#nodealpine to understand why libc6-compat might be needed.
    RUN apk add --no-cache libc6-compat

FROM base AS builder
    WORKDIR /app

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

    RUN turbo run build --filter=api...

FROM base AS runner
    WORKDIR /app

    RUN npm install -g pnpm

    # Don't run production as root
    RUN addgroup --system --gid 1001 expressjs
    RUN adduser --system --uid 1001 expressjs

    USER expressjs
    COPY --from=installer /app .

    # TODO: Maybe use the npm script?
    CMD pnpm --filter api run start
