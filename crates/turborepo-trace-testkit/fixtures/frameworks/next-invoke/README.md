# next-invoke

A Next.js app whose pages reach route handlers through common data-fetching idioms: a fetch
wrapper that adds an origin and an `/api` prefix behind `startsWith` guards and calls `fetch`
from a callback in a second function (`lib/http/client.ts`), a typed client that substitutes
`{param}` placeholders in a loop and is imported through a barrel (`lib/http/typed/`), SWR
hooks with inline fetchers, wrapper hooks that forward a key and a fetcher (`lib/http/hooks.ts`),
URL constants (`lib/http/routes.ts`), server actions, TanStack Query hooks, tRPC procedures,
redirects and navigations. All names are made up.

`crates/turborepo-trace-frameworks/tests/next_invoke.rs` snapshots the sites and the per-route relation.
