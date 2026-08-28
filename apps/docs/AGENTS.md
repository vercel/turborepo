# Turborepo docs app

The root `AGENTS.md` also applies to this app. Geistdocs owns shared runtime behavior, while this app owns Turborepo content, routes, metadata, assets, and adapters.

## Geistdocs

- Read `node_modules/@vercel/geistdocs/docs/agents.md` and its focused package docs before changing package-backed behavior.
- Keep `createGeistdocs` as the outer Next.js wrapper. Keep `cacheComponents` and `partialPrefetching` enabled.
- Do not export `dynamic`, `revalidate`, `fetchCache`, or `dynamicParams` from App Router files. Use `use cache` and `cacheLife` for app-owned caching.
- Read `[lang]` through `next/root-params` in Server Components. Keep route-handler and Server Action `params` in their context arguments.
- Use `prefetch={true}` for app-owned links to statically generated documentation pages.
- Restart `next dev` after adding, deleting, or renaming App Router pages or route handlers.
- Do not edit `.source`, `.next`, `node_modules`, or package internals.

## Public contracts

- `/llms.txt` is the page index. `/llms-full.txt` is the complete docs corpus.
- Preserve page Markdown, content negotiation, `/agents.md`, `/sitemap.md`, both RSS feeds, XML sitemap state, robots rules, search, Ask AI, signed OG routes, schemas, and proxy exclusions.
- Static app-owned routes serve `/terms` and `/governance`. The localized not-found page owns browser 404s, while Geistdocs owns automatic agent-readable 404 recovery.
- Keep blog, OpenAPI, showcase, Devtools, redirects, version-host behavior, analytics, and the Remote Cache page action app-owned.
- Do not advertise an MCP server unless Turborepo provides one.

<!-- BEGIN:nextjs-agent-rules -->

# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->
