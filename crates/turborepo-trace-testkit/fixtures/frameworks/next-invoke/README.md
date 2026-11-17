# next-invoke

A Next.js app whose pages reach route handlers through the data-fetching idioms of real apps:
a fetch wrapper that adds a base and an `/api` prefix behind `startsWith` guards and calls
`fetch` from a callback in a second function (`lib/api/fetch-api.ts`), a typed client that
substitutes `{param}` placeholders in a loop and is imported through a barrel
(`lib/api/typed/`), SWR hooks with inline fetchers, wrapper hooks that forward a key and a
fetcher (`lib/api/swr.ts`), URL constants (`lib/api/endpoints.ts`), server actions, TanStack
Query hooks, tRPC procedures, redirects and navigations. The shapes come from a large
production app; the names do not.

`crates/turborepo-trace-frameworks/tests/next_invoke.rs` snapshots the sites and the per-route relation.
