# Turborepo Docker starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-docker
```

## What's inside?

This Turborepo includes the following:

### Apps and Packages

- `web`: a [Next.js](https://nextjs.org/) app
- `api`: an [Express](https://expressjs.com/) server
- `@repo/ui`: a React component library
- `@repo/logger`: Isomorphic logger (a small wrapper around `console.log`)
- `@repo/eslint-config`: ESLint presets
- `@repo/typescript-config`: shared TypeScript configurations
- `@repo/jest-presets`: Jest configurations

Each package and app is written in [TypeScript](https://www.typescriptlang.org/).

### Docker

This repo is configured to be built with Docker Compose. To build all apps in this repo:

```sh
# Install dependencies
yarn install --frozen-lockfile

# Create a network that allows containers to communicate using their
# container names as hostnames
docker network create app_network

# Build the production images
docker compose build

# Start production in detached mode
docker compose up -d
```

Open http://localhost:3000.

To shut down all running containers:

```sh
docker compose down
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

This example includes optional remote caching. In the Dockerfiles of the apps, uncomment the build arguments for `TURBO_TEAM` and `TURBO_TOKEN`. Then, pass these build arguments to your Docker build.

You can test this behavior using a command like:

```sh
docker build -f apps/web/Dockerfile . --build-arg TURBO_TEAM="your-team-name" --build-arg TURBO_TOKEN="your-token" --no-cache
```

### Utilities

This Turborepo has some additional tools already set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Jest](https://jestjs.io) test runner for all things JavaScript
- [Prettier](https://prettier.io) for code formatting
