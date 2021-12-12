## What's inside?

This turborepo uses [Yarn](https://classic.yarnpkg.com/lang/en/) as a package manager. It includes the following packages:

### Apps and Packages

- `hello world`: a npm package

Each package/app is 100% [Typescript](https://www.typescriptlang.org/).

### Utilities

This turborepo has some additional tools already setup for you:

- [Typescript](https://www.typescriptlang.org/) for static type checking
- [SWC](https://swc.rs/) for building your code faster
- [Jest](https://jestjs.io) test runner for all things JavaScript

### Build

To build all packages, run the following command:

```
cd my-turborepo
yarn run build
```

### Develop

To develop all packages, run the following command:

```
cd my-turborepo
yarn run dev
```

### Publish

To publish modified packages, run the following command:

```
cd my-turborepo
yarn run release
```

_This command builds packages before publishing._

### Remote Caching

Turborepo can use a technique known as [Remote Caching (Beta)](https://turborepo.org/docs/features/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching (Beta) you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup), then enter the following commands:

```
cd my-turborepo
npx turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, you can link your Turborepo to your Remote Cache by running the following command from the root of your turborepo:

```
npx turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Pipelines](https://turborepo.org/docs/features/pipelines)
- [Caching](https://turborepo.org/docs/features/caching)
- [Remote Caching (Beta)](https://turborepo.org/docs/features/remote-caching)
- [Scoped Tasks](https://turborepo.org/docs/features/scopes)
- [Configuration Options](https://turborepo.org/docs/reference/configuration)
- [CLI Usage](https://turborepo.org/docs/reference/command-line-reference)