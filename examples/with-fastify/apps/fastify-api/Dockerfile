FROM node:18-alpine AS base

FROM base AS builder

RUN apk update
RUN apk add --no-cache libc6-compat

WORKDIR /app
RUN npm install turbo --global
COPY . .
RUN turbo prune fastify-api --docker

FROM base AS installer
RUN apk update
RUN apk add --no-cache libc6-compat
WORKDIR /app

COPY --from=builder /app/out/json/ .
RUN npm install

COPY --from=builder /app/out/full/ .

# Uncomment and use build args to enable remote caching
# ARG TURBO_TEAM
# ENV TURBO_TEAM=$TURBO_TEAM

# ARG TURBO_TOKEN
# ENV TURBO_TOKEN=$TURBO_TOKEN

RUN npm install turbo --global
RUN turbo build

FROM base AS runner
WORKDIR /app

# Don't run production as root
RUN addgroup --system --gid 1001 fastify
RUN adduser --system --uid 1001 fastify
USER fastify
COPY --from=installer /app .

EXPOSE 3002

CMD node apps/fastify-api/dist/index.js