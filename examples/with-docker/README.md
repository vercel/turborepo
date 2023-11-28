# Turborepo Docker starter

This is an official Docker starter Turborepo.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-docker
```

## What's inside?

This turborepo uses [Yarn](https://classic.yarnpkg.com/lang/en/) as a package manager. It includes the following packages/apps:

### Apps and Packages

- `@repo/web`: a [Next.js](https://nextjs.org/) app
- `@repo/api`: an [Express](https://expressjs.com/) server
- `@repo/ui`: ui: a React component library
- `@repo/eslint-config-custom`: `eslint` configurations for client side applications (includes `eslint-config-next` and `eslint-config-prettier`)
- `@repo/eslint-config-custom-server`: `eslint` configurations for server side applications (includes `eslint-config-next` and `eslint-config-prettier`)
- `scripts`: Jest configurations
- `@repo/logger`: Isomorphic logger (a small wrapper around console.log)
- `@repo/typescript-config`: tsconfig.json's used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Docker

This repo is configured to be built with Docker, and Docker compose. To build all apps in this repo:

```
# Create a network, which allows containers to communicate
# with each other, by using their container name as a hostname
docker network create app_network

# Build prod using new BuildKit engine
COMPOSE_DOCKER_CLI_BUILD=1 DOCKER_BUILDKIT=1 docker-compose -f docker-compose.yml build

# Start prod in detached mode
docker-compose -f docker-compose.yml up -d
```

Open http://localhost:3000.

To shutdown all running containers:

```
# Stop all running containers
docker kill $(docker ps -q) && docker rm $(docker ps -a -q)
```

### Remote Caching

This example includes optional remote caching. In the Dockerfiles of the apps, uncomment the build arguments for `TURBO_TEAM` and `TURBO_TOKEN`. Then, pass these build arguments to your Docker build.

You can test this behavior using a command like:

`docker build -f apps/web/Dockerfile . --build-arg TURBO_TEAM=“your-team-name” --build-arg TURBO_TOKEN=“your-token“ --no-cache`

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Jest](https://jestjs.io) test runner for all things JavaScript
- [Prettier](https://prettier.io) for code formatting
