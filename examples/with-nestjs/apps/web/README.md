# Getting Started

From the monorepo root, install dependencies with `pnpm install`, then run:

```bash
pnpm dev
```

This starts both the Next.js frontend and NestJS API, building shared packages first.

Browse [localhost:3001](http://localhost:3001) to see the result. The API runs at [localhost:3000](http://localhost:3000).

You can start editing the page by modifying `app/page.tsx`. The page auto-updates as you edit the file.

This project uses [`next/font/local`](https://nextjs.org/docs/app/getting-started/fonts#local-fonts) to optimize the bundled Geist Sans and Geist Mono fonts without fetching Google Fonts during builds.

## Learn More

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js features and APIs.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.
- [Next.js GitHub repository](https://github.com/vercel/next.js).

## Deploy on Vercel

The easiest way to deploy the frontend is to use the [Vercel Platform](https://vercel.com/new?utm_medium=default-template&filter=next.js&utm_source=create-next-app&utm_campaign=create-next-app-readme).

See the [Next.js deployment documentation](https://nextjs.org/docs/app/getting-started/deploying) for more details. Deploy the NestJS API separately and configure the frontend's API URL in `app/page.tsx` for your deployment.
