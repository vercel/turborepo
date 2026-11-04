## Getting Started

Install dependencies from the monorepo root using the Node.js and pnpm versions in the [root README](../../README.md). Then start this app and its shared package watchers from that root:

```bash
pnpm exec turbo run dev --filter=web...
```

Open [http://localhost:3001](http://localhost:3001) with your browser to see the result.

You can start editing the page by modifying `app/page.tsx`. The page auto-updates as you edit the file.

To create [Route Handlers](https://nextjs.org/docs/app/api-reference/file-conventions/route), add a `route.ts` file to an `app/` subdirectory. For example, `app/api/hello/route.ts` maps to [http://localhost:3001/api/hello](http://localhost:3001/api/hello).

## Learn More

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js features and APIs.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.

## Deploy on Vercel

Deploy with the [Vercel Platform](https://vercel.com/new?utm_source=github.com&utm_medium=referral&utm_campaign=turborepo-readme).

See the [Next.js deployment documentation](https://nextjs.org/docs/app/getting-started/deploying) for more details.
