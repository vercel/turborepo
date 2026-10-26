# With-NestJS | API

## Getting Started

From the monorepo root, install dependencies with `pnpm install`, then run:

```bash
pnpm turbo run dev --filter=api...
```

Turborepo builds shared packages before starting the development servers and watches the shared API package for changes.

By default, the API runs at [localhost:3000](http://localhost:3000). You can use an API client like [Insomnia](https://insomnia.rest/) or [Postman](https://www.postman.com/) to test it.

You can start editing the demo APIs by modifying the [links service](./src/links/links.service.ts).

To build or test only the API, run these commands from the monorepo root. Turborepo builds the required shared packages first:

```bash
pnpm turbo run build --filter=api
pnpm turbo run test test:e2e --filter=api
```

## Learn More

- [Official Documentation](https://docs.nestjs.com) - A progressive Node.js framework for building efficient, reliable and scalable server-side applications.
- [Official NestJS Courses](https://courses.nestjs.com) - Learn everything you need to master NestJS and tackle modern backend applications at any scale.
- [GitHub Repo](https://github.com/nestjs/nest)
