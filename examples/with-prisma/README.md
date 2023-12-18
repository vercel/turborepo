# Turborepo starter

This is an official starter turborepo.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-prisma
```

## What's inside?

This turborepo includes the following packages:

### Packages

- `web`: a [Next.js](https://nextjs.org/) app
- `@repo/eslint-config`: `eslint` configurations (includes `eslint-config-next` and `eslint-config-prettier`)
- `@repo/database`: [Prisma](https://prisma.io/) ORM wrapper to manage & access your database
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo

Each package is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This turborepo additionally sets up:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for linting
- [Prettier](https://prettier.io) for code formatting
- [Prisma](https://prisma.io/) for database ORM
- [Docker Compose](https://docs.docker.com/compose/) for local database

### Database

We use [Prisma](https://prisma.io/) to manage & access our database. You will
need a database either locally or hosted in the cloud for this project.

We offer a [`docker-compose.yml`][1] configuration to set up a MySQL server
locally with a new database named `turborepo`:

```bash
cd my-turborepo
docker-compose up -d
```

You can customize the name of the database by setting the `MYSQL_DATABASE`
environment variable in the [`docker-compose.yml`](./docker-compose.yml).

Once deployed, copy the `.env.example` file to `.env` so Prisma has a
`DATABASE_URL` environment variable to access.

```bash
cp .env.example .env
```

Update `DATABASE_URL` in your `.env` if you changed the database name or are
using a cloud-based database.

Once deployed & up & running, create & deploy migrations to your database to add
the necessary tables. This can be done using
[Prisma Migrate](https://www.prisma.io/migrate):

```bash
npx prisma migrate dev
```

Push migrations to the database with

```bash
yarn run db:push
# OR
yarn run db:migrate:deploy
```

See Prisma docs on [the difference][2] between the two commands!

You can add seed data to your database using Prisma's [seeding][3]
functionality. To do this update the seed script located in
[`packages/database/src/seed.ts`][4] & add or update any users you wish to seed
to the database.

Then run the seed script with:

```bash
yarn run db:seed
```

For further more information on migrations, seeding & more, read through the
[Prisma Documentation](https://www.prisma.io/docs/).

### Build

To build all apps and packages, run the following command:

```bash
yarn run build
```

### Develop

To develop all apps and packages, run the following command:

```bash
yarn run dev
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turbo.build/repo/docs/core-concepts/monorepos/running-tasks)
- [Caching](https://turbo.build/repo/docs/core-concepts/caching)
- [Remote Caching](https://turbo.build/repo/docs/core-concepts/remote-caching)
- [Filtering](https://turbo.build/repo/docs/core-concepts/monorepos/filtering)
- [Configuration Options](https://turbo.build/repo/docs/reference/configuration)
- [CLI Usage](https://turbo.build/repo/docs/reference/command-line-reference)

[1]: https://docs.docker.com/compose/
[2]: https://www.prisma.io/docs/concepts/components/prisma-migrate/db-push#choosing-db-push-or-prisma-migrate
[3]: https://www.prisma.io/docs/guides/database/seed-database

[4]: [./packages/database/src/seed.ts]
