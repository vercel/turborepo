# Turborepo + Prisma ORM starter

This example helps you quickly set up a Turborepo monorepo with a Next.js app and Prisma ORM. It is community maintained. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## What's inside?

This Turborepo includes the following packages and apps:

- `web`: a [Next.js](https://nextjs.org/) app
- `@repo/eslint-config`: shared ESLint flat configurations
- `@repo/database`: [Prisma ORM](https://prisma.io/) for database access
- `@repo/typescript-config`: shared `tsconfig.json` files

The example also uses TypeScript, Prettier, PostgreSQL, and Docker Compose.

## Getting started

### 1. Create the project

```sh
pnpm dlx create-turbo@latest -e with-prisma
cd my-turborepo
```

### 2. Start PostgreSQL

The included [`docker-compose.yml`](./docker-compose.yml) starts a local PostgreSQL server with a database named `turborepo`:

```sh
docker compose up -d
```

To change the database name, update `POSTGRES_DB` in [`docker-compose.yml`](./docker-compose.yml).

### 3. Configure environment variables

Copy the example environment file into the database package and web app:

```sh
cp .env.example packages/database/.env
cp .env.example apps/web/.env
```

Update `DATABASE_URL` in those files if you changed the database name or use a hosted database.

### 4. Create the database schema

Emit the Prisma ORM contract, then initialize an empty database:

```sh
pnpm generate
pnpm --filter @repo/database exec prisma db init
```

After editing the contract, apply development changes with `pnpm db:push`. Use `pnpm --filter @repo/database exec prisma migration plan` to create a migration and `pnpm db:migrate:deploy` to apply committed migrations.

### 5. Seed the database

Edit [`packages/database/src/seed.ts`](./packages/database/src/seed.ts), then run:

```sh
pnpm db:seed
```

### 6. Build and run the application

```sh
pnpm build
pnpm dev
```

Open `http://localhost:3000` in your browser.

For a detailed walkthrough, see the [Prisma ORM documentation](https://www.prisma.io/docs/orm).

## Useful links

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration options](https://turborepo.dev/docs/reference/configuration)
- [CLI usage](https://turborepo.dev/docs/reference/command-line-reference)
