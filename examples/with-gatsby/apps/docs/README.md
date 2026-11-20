## Getting Started

Install dependencies from the workspace root, then start this app:

```sh
pnpm --filter docs dev
```

Open [http://localhost:3001](http://localhost:3001). Edit `pages/index.tsx` to see updates. This app uses the Next.js Pages Router and consumes `@repo/ui` through `transpilePackages`.

## Production

From the workspace root:

```sh
pnpm --filter docs build
pnpm --filter docs start
```

The production server also uses port 3001.

## Learn More

- [Next.js Pages Router documentation](https://nextjs.org/docs/pages)
- [Next.js deployment documentation](https://nextjs.org/docs/pages/getting-started/deploying)
