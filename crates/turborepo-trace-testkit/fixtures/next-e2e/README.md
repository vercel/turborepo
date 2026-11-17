# next-e2e

A small Next.js App Router app with Playwright specs, for `turbotrace affected --tests playwright`
from source: which specs a changed component reaches.

| Route | Entry files | Loads |
|---|---|---|
| `/` | `app/layout.tsx`, `app/page.tsx` | `SiteHeader`; `Hero` through the barrel `components/index.ts` |
| `/pricing` | `app/layout.tsx`, `app/pricing/page.tsx` | `PricingTable` (uses `Button`), `PlanCalculator` through `next/dynamic` |
| `/dashboard` | + `app/dashboard/layout.tsx`, `page.tsx` | `Sidebar`; `StatsCard`, which fetches `/api/stats` |
| `/dashboard/settings` | + `app/dashboard/settings/page.tsx` | `SettingsForm` (uses `Button`, the server action `saveSettings`) |
| `/blog/[slug]` | `app/layout.tsx`, `app/blog/[slug]/page.tsx` | `PostBody` through the barrel |
| `/blog/archive` | `app/layout.tsx`, `app/blog/archive/page.tsx` | `ArchiveList` |
| `/api/stats` | `app/api/stats/route.ts` | `lib/stats.ts` |

`next.config.mjs` redirects `/plans` to `/pricing`. `components/badge.tsx` is exported by the barrel
and used by no route.

| Spec | Visits |
|---|---|
| `e2e/home.spec.ts` | `/` |
| `e2e/pricing.spec.ts` | `/pricing`, and `/plans` (the redirect) |
| `e2e/dashboard.spec.ts` | `/dashboard`, `/dashboard/settings` through the page object `e2e/pages/dashboard-page.ts` |
| `e2e/blog.spec.ts` | `/blog/hello-world` (a constant in a template) |
| `e2e/api.spec.ts` | `GET /api/stats` with the request fixture |

The app is not built or run by the tests: selection reads the source.
