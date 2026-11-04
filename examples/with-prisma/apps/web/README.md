## Getting Started

Follow the [root setup instructions](../../README.md) first to install dependencies, start PostgreSQL, configure both `.env` files, initialize the database, and seed it.

From the repository root, run the development tasks (including contract generation):

```sh
pnpm dev
```

Open [http://localhost:3000](http://localhost:3000) with your browser to see the result. The page queries the database on each request.

You can start editing the page by modifying `app/page.tsx`. The page auto-updates as you edit the file.

To create [Route Handlers](https://nextjs.org/docs/app/building-your-application/routing/route-handlers), add a `route.ts` file under `app/`. For example, `app/api/hello/route.ts` maps to [http://localhost:3000/api/hello](http://localhost:3000/api/hello).

## Learn More

- [Next.js Documentation](https://nextjs.org/docs)
- [Learn Next.js](https://nextjs.org/learn)
- [Next.js GitHub repository](https://github.com/vercel/next.js/)

## Deploy on Vercel

Deploy with [Vercel](https://vercel.com/new) or follow the [Next.js deployment documentation](https://nextjs.org/docs/app/getting-started/deploying). Configure `DATABASE_URL` for a reachable PostgreSQL database, and apply committed database migrations before serving requests. Never use the local Compose credentials in production.
