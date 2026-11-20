# Turborepo + Prisma ORM starter

This example helps you quickly set up a Turborepo monorepo with a Next.js app and Prisma ORM. It is community maintained. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## What's inside?

This Turborepo includes the following packages and apps:

- `web`: a [Next.js](https://nextjs.org/) app
- `@repo/eslint-config`: shared ESLint flat configurations
- `@repo/database`: [Prisma ORM](https://prisma.io/) for database access
- `@repo/typescript-config`: shared `tsconfig.json` files

The example also uses TypeScript, Prettier, PostgreSQL, and Docker Compose. Prisma's current npm `latest` release is a Prisma ORM 8 release candidate, which uses contracts and the `db`/`migration` commands rather than the Prisma ORM 7 client workflow.

## Getting started

### 1. Create the project

Use Node.js 26.12.1 or newer and pnpm 12.14.0 (pinned in `package.json`), and install Docker with Compose support.

```sh
pnpm dlx create-turbo@latest my-turborepo -e with-prisma
cd my-turborepo
pnpm install
```

### 2. Start PostgreSQL

The included [`docker-compose.yml`](./docker-compose.yml) starts PostgreSQL 18.6 with a database named `turborepo`:

```sh
docker compose up -d
```

To change the database name, update `POSTGRES_DB` in [`docker-compose.yml`](./docker-compose.yml). The named `database` volume persists data at PostgreSQL 18's `/var/lib/postgresql` mount point. Existing databases from older PostgreSQL major versions require a dump/restore or `pg_upgrade`; do not mount an older data directory directly into this container.

### 3. Configure environment variables

Copy the example environment file into the database package and web app:

```sh
cp .env.example packages/database/.env
cp .env.example apps/web/.env
```

The example URL matches the local Compose credentials and database. Update `DATABASE_URL` in both files if you changed them or use a hosted database. These credentials are for local development only.

### 4. Create the database schema

Emit the Prisma ORM contract, then initialize an empty database:

```sh
pnpm generate
pnpm --filter @repo/database exec prisma db init
```

After editing the contract, apply development changes with `pnpm db:push` (also available as `pnpm db:migrate:dev`). Use `pnpm --filter @repo/database exec prisma migration plan` to create a migration and `pnpm db:migrate:deploy` to apply committed migrations. Database tasks always run uncached and generate the contract first.

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

Contract generation is a Turbo dependency of build and development tasks, including when generated files are restored from cache. The web page queries PostgreSQL at request time, so the database must be running to view it.

Open `http://localhost:3000` in your browser.

Run `pnpm lint` for Next.js and environment-variable lint rules, and `pnpm check-types` for TypeScript checks. The shared flat ESLint configuration uses Babel to parse TypeScript without depending on the legacy TypeScript compiler API; the TypeScript 7 compiler checks types separately.

For a detailed walkthrough, see the [Prisma ORM documentation](https://www.prisma.io/docs/orm).

## Useful links

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration options](https://turborepo.dev/docs/reference/configuration)
- [CLI usage](https://turborepo.dev/docs/reference/command-line-reference)
