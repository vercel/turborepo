# next-invoke-locale

A Next.js app whose pages live under `/[locale]` and whose middleware rewrites `/x` to
`/<locale>/x` (the shape of a public scheduling app). The code names its own pages without the
locale: `redirect('/login')`, `router.push('/login/verify')`, `router.push(`/poll/${id}`)`. The
target of a link or a redirect is requested like any URL, so it is matched behind the segment
the middleware puts in front.

| Site | Target |
|---|---|
| `app/[locale]/login/verify/page.tsx`: `redirect('/login')` | `/[locale]/login` |
| `components/login-form.tsx`: `router.push('/login/verify')` | `/[locale]/login/verify` |
| `components/create-poll.tsx`: `router.push(`/poll/${id}`)` | `/[locale]/poll/[id]`; `/embed/poll/[id]` only if the middleware put the fixed segment `embed` in front (a wide target) |
| `lib/loaders.ts`: `redirect('/admin-setup')` in the loader the admin layout calls | `/[locale]/admin-setup`, for every route below the layout |
| `app/[locale]/page.tsx`: `<a href="/docs/intro">` | `/docs/[slug]`: the middleware's matcher leaves `/docs` alone |
| `components/nav.tsx`: `<Link href="/new">` with the app's own `Link` (`components/link.tsx` hands its props to `next/link`), `<Button href="/login">` of a package | `/[locale]/new`, `/[locale]/login`: the element with the `href` is the site |

The middleware is a library's (`createMiddleware`): its file has no redirect or rewrite, so
nothing says a request can end anywhere, and wide targets are listed. A test replaces it with
one that builds its rewrite target from the request: then a request can end on any page and
wide targets are not listed.

`crates/turborepo-trace-frameworks/tests/next_invoke.rs` checks the targets.
