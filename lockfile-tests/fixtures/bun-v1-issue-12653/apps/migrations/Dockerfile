FROM oven/bun:1.3.13-alpine AS base

WORKDIR /app

FROM base AS pruner

RUN bun add -g turbo@2.9.7-canary.13

COPY . .

RUN turbo prune @eventmate/migrations --docker

FROM base AS installer

COPY --from=pruner /app/out/json/ .

RUN bun install --frozen-lockfile --ignore-scripts

COPY --from=pruner /app/out/full/ .

RUN cd packages/database && bun run generate

FROM base AS runner

COPY --from=installer /app ./

RUN chmod +x ./apps/migrations/docker-entrypoint.sh

WORKDIR /app/apps/migrations

CMD ["./docker-entrypoint.sh"]
