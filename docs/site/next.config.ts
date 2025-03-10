import { createMDX } from "fumadocs-mdx/next";
import { withVercelToolbar } from "@vercel/toolbar/plugins/next";
import { REDIRECTS_FROM_ADDING_PACK } from "./lib/redirects/add-pack-docs.mjs";
import { REDIRECTS_FOR_V2_DOCS } from "./lib/redirects/v2-docs.mjs";

const withMDX = createMDX();
const vercelToolbar = withVercelToolbar();

const config = {
  experimental: {
    mdxRs: true,
  },
  reactStrictMode: true,
  images: {
    formats: ["image/avif", "image/webp"],
    minimumCacheTTL: 1800,
  },
  typescript: {
    ignoreBuildErrors: true,
  },
  eslint: {
    ignoreDuringBuilds: true,
  },
  rewrites() {
    return {
      beforeFiles:
        process.env.VERCEL_ENV === "production"
          ? [
              {
                source: "/sitemap.xml",
                destination:
                  "https://crawled-sitemap.vercel.sh/turbobuild-sitemap.xml",
              },
              {
                source: "/api/feedback",
                destination: "https://vercel.com/api/feedback",
              },
            ]
          : undefined,
    };
  },
  redirects() {
    return [
      ...REDIRECTS_FROM_ADDING_PACK.map((route) => ({
        source: route,
        destination: `/repo${route}`,
        permanent: true,
      })),
      {
        source: "/docs/getting-started",
        destination: "/repo/docs",
        permanent: true,
      },
      {
        source: "/usage",
        destination: "/repo/docs/reference/command-line-reference",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/running-tasks",
        destination: "/repo/docs/core-concepts/monorepos/running-tasks",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/why-turborepo",
        destination: "/repo/docs/core-concepts/monorepos",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/filtering",
        destination: "/repo/docs/core-concepts/monorepos/filtering",
        permanent: true,
      },
      {
        source: "/docs/guides/workspaces",
        destination: "/repo/docs/handbook/workspaces",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/workspaces",
        destination: "/repo/docs/handbook/workspaces",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/pipelines",
        destination: "/repo/docs/core-concepts/monorepos/running-tasks",
        permanent: true,
      },
      {
        source: "/docs/guides/migrate-from-lerna",
        destination: "/repo/docs/handbook/migrating-to-a-monorepo",
        permanent: true,
      },
      {
        source: "/discord{/}?",
        destination: "https://vercel.community/tag/turborepo",
        permanent: true,
      },
      {
        source: "/docs/changelog",
        destination: "https://github.com/vercel/turbo/releases",
        permanent: true,
      },
      {
        source: "/docs/guides/complimentary-tools",
        destination: "/repo/docs/handbook",
        permanent: true,
      },
      {
        source: "/docs/guides/monorepo-tools",
        destination: "/repo/docs/handbook",
        permanent: true,
      },
      {
        source: "/docs/glossary",
        destination: "/repo/docs/handbook",
        permanent: true,
      },
      {
        source: "/docs/guides/continuous-integration",
        destination: "/repo/docs/ci",
        permanent: true,
      },
      {
        source: "/repo/docs/handbook/prisma",
        destination: "/repo/docs/handbook/tools/prisma",
        permanent: true,
      },
      {
        source: "/pack/docs/comparisons/turbopack-vs-vite",
        destination: "/pack/docs/comparisons/vite",
        permanent: true,
      },
      {
        source: "/pack/docs/comparisons/turbopack-vs-webpack",
        destination: "/pack/docs/comparisons/webpack",
        permanent: true,
      },
      {
        source: "/pack/docs/features/customizing-turbopack",
        destination:
          "https://nextjs.org/docs/app/api-reference/next-config-js/turbo",
        permanent: true,
      },
      {
        source: "/repo/docs/platform-environment-variables",
        destination:
          "/repo/docs/crafting-your-repository/using-environment-variables#platform-environment-variables",
        permanent: true,
      },
      ...REDIRECTS_FOR_V2_DOCS.map((route) => ({
        source: route.source,
        destination: route.destination,
        permanent: true,
      })),
      {
        source: "/pack/docs/core-concepts",
        destination: "/pack/docs/incremental-computation",
        permanent: true,
      },
      // March 4, 2025: Removal of Turbopack from these docs
      {
        source: "/pack/:slug*",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        // Redirect old blog posts to new blog.
        source: "/posts/:path*",
        destination: "/blog/:path*",
        permanent: true,
      },
      // OpenAPI redirects (until we have more content)
      {
        source: "/repo/docs/openapi",
        destination: "/repo/docs/openapi/artifacts/artifact-exists",
        permanent: false,
      },
      {
        source: "/repo/docs/openapi/artifacts",
        destination: "/repo/docs/openapi/artifacts/artifact-exists",
        permanent: false,
      },
    ];
  },
};

// @ts-expect-error -- Not sure what's up here but not worth spending time on.
// eslint-disable-next-line import/no-default-export
export default withMDX(vercelToolbar(config));
