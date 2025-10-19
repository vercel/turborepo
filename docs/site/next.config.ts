import type { NextConfig } from "next";
import { createMDX } from "fumadocs-mdx/next";
import { withVercelToolbar } from "@vercel/toolbar/plugins/next";
import { REDIRECTS_FOR_V2_DOCS } from "./lib/redirects/v2-docs.mjs";

const withMDX = createMDX();
const vercelToolbar = withVercelToolbar();

const llmMarkdownRedirects = {
  source: "/docs/:path*.md",
  destination: "/llms.md/:path*",
};

const config: NextConfig = {
  reactStrictMode: true,
  images: {
    formats: ["image/avif", "image/webp"],
    remotePatterns: [
      {
        protocol: "https",
        hostname: "ufa25dqjajkmio0q.public.blob.vercel-storage.com",
      },
      {
        protocol: "https",
        hostname: "x.com",
      },
    ],
    minimumCacheTTL: 1800,
  },
  typescript: {
    ignoreBuildErrors: true,
  },
  eslint: {
    ignoreDuringBuilds: true,
  },
  // Next.js still expects these to return Promises even without await
  // eslint-disable-next-line @typescript-eslint/require-await -- Purposeful.
  async rewrites() {
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
              llmMarkdownRedirects,
            ]
          : [llmMarkdownRedirects],
    };
  },
  // Next.js still expects these to return Promises even without await
  // eslint-disable-next-line @typescript-eslint/require-await -- Purposeful.
  async redirects() {
    return [
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
        destination: "https://community.vercel.com/tag/turborepo",
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
      // March 4, 2025: Removal of Turbopack from these docs
      {
        source: "/pack/:slug*",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/blog/turbopack-benchmarks",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        // Redirect old blog posts to new blog.
        source: "/posts/:path*",
        destination: "/blog/:path*",
        permanent: true,
      },
      {
        source: "/repo/docs/:slug*",
        destination: "/docs/:slug*",
        permanent: true,
      },
      // OpenAPI redirects (until we have more content)
      {
        source: "/docs/openapi",
        destination: "/repo/docs/openapi/artifacts/artifact-exists",
        permanent: false,
      },
      {
        source: "/docs/openapi/artifacts",
        destination: "/repo/docs/openapi/artifacts/artifact-exists",
        permanent: false,
      },
      {
        source: "/docs/getting-started/support-policy",
        destination: "/docs/support-policy",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/monorepos/filtering",
        destination:
          "/docs/crafting-your-repository/running-tasks#using-filters",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/monorepos/running-tasks",
        destination: "/docs/crafting-your-repository/running-tasks",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/caching",
        destination: "/docs/crafting-your-repository/caching",
        permanent: true,
      },
      {
        // Slug pattern that was historically used for blog and nothing else
        source: "/turbo-:slug",
        destination: "/blog/turbo-:slug",
        permanent: false,
      },
      {
        source: "/benchmarks",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/cookie-policy",
        destination: "https://vercel.com/legal/privacy-policy",
        permanent: true,
      },
      {
        source: "/core-concepts",
        destination: "/docs/core-concepts",
        permanent: true,
      },
      {
        source: "/core-concepts/internal-packages",
        destination: "/docs/core-concepts/internal-packages",
        permanent: true,
      },
      {
        source: "/core-concepts/package-and-task-graph",
        destination: "/docs/core-concepts/package-and-task-graph",
        permanent: true,
      },
      {
        source: "/core-concepts/remote-caching",
        destination: "/docs/core-concepts/remote-caching",
        permanent: true,
      },
      {
        source: "/crafting-your-repository",
        destination: "/docs/crafting-your-repository",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/caching",
        destination: "/docs/crafting-your-repository/caching",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/configuring-tasks",
        destination: "/docs/crafting-your-repository/configuring-tasks",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/developing-applications",
        destination: "/docs/crafting-your-repository/developing-applications",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/managing-dependencies",
        destination: "/docs/crafting-your-repository/managing-dependencies",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/running-tasks",
        destination: "/docs/crafting-your-repository/running-tasks",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/structuring-a-repository",
        destination: "/docs/crafting-your-repository/structuring-a-repository",
        permanent: true,
      },
      {
        source: "/crafting-your-repository/using-environment-variables",
        destination:
          "/docs/crafting-your-repository/using-environment-variables",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/scopes",
        destination: "/docs/crafting-your-repository/running-tasks",
        permanent: true,
      },
      {
        source: "/docs/features/scopes",
        destination: "/docs/crafting-your-repository/running-tasks",
        permanent: true,
      },
      {
        source: "/docs/features/caching",
        destination: "/docs/crafting-your-repository/caching",
        permanent: true,
      },
      {
        source: "/docs/features/pipelines",
        destination: "/docs/crafting-your-repository/running-tasks",
        permanent: true,
      },
      {
        source: "/docs/features/remote-caching",
        destination: "/docs/core-concepts/remote-caching",
        permanent: true,
      },
      {
        source: "/docs/installation",
        destination: "/docs/getting-started/installation",
        permanent: true,
      },
      {
        source: "/docs/llms.txt",
        destination: "/llms.txt",
        permanent: true,
      },
      {
        source: "/features",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/features/:path*",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/free-vercel-remote-cache",
        destination: "/blog/free-vercel-remote-cache",
        permanent: true,
      },
      {
        source: "/guides/ci-vendors",
        destination: "/docs/guides/ci-vendors",
        permanent: true,
      },
      {
        source: "/guides/ci-vendors/:path*",
        destination: "/docs/guides/ci-vendors/:path*",
        permanent: true,
      },
      {
        source: "/guides/generating-code",
        destination: "/docs/guides/generating-code",
        permanent: true,
      },
      {
        source: "/guides/single-package-workspaces",
        destination: "/docs/guides/single-package-workspaces",
        permanent: true,
      },
      {
        source: "/guides/tools",
        destination: "/docs/guides/tools",
        permanent: true,
      },
      {
        source: "/guides/tools/:path*",
        destination: "/docs/guides/tools/:path*",
        permanent: true,
      },
      {
        source: "/joining-vercel",
        destination: "/blog/joining-vercel",
        permanent: true,
      },
      {
        source: "/migrating-from-webpack",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/pack/docs/features/imports",
        destination: "/docs/crafting-your-repository/structuring-a-repository",
        permanent: true,
      },
      {
        source: "/reference",
        destination: "/docs/reference",
        permanent: true,
      },
      {
        source: "/reference/package-configurations",
        destination: "/docs/reference/package-configurations",
        permanent: true,
      },
      {
        source: "/reference/run",
        destination: "/docs/reference/run",
        permanent: true,
      },
      {
        source: "/repo/docs/core-concepts/pipelines",
        destination: "/docs/crafting-your-repository/running-tasks",
        permanent: true,
      },
      {
        source: "/repo/docs/getting-started",
        destination: "/docs/getting-started",
        permanent: true,
      },
      {
        source: "/repo/docs/getting-started/introduction",
        destination: "/docs/getting-started",
        permanent: true,
      },
      {
        source: "/repo/docs/reference/command-line-reference",
        destination: "/docs/reference",
        permanent: true,
      },
      {
        source: "/repo/docs/reference/gen",
        destination: "/docs/reference/generate",
        permanent: true,
      },
      {
        source: "/docs/reference/gen",
        destination: "/docs/reference/generate",
        permanent: true,
      },
      {
        source: "/roi-calculator",
        destination: "/docs",
        permanent: true,
      },
      {
        source: "/telemetry",
        destination: "/docs/telemetry",
        permanent: true,
      },
      {
        source: "/docs/intro",
        destination: "/docs",
        permanent: true,
      },
      {
        source: "/getting-started/add-to-existing-repository",
        destination: "/docs/getting-started/add-to-existing-repository",
        permanent: true,
      },
      {
        source: "/getting-started/installation",
        destination: "/docs/getting-started/installation",
        permanent: true,
      },
      {
        source: "/guides/publishing-libraries",
        destination: "/docs/guides/publishing-libraries",
        permanent: true,
      },
      {
        source: "/guides/skipping-tasks",
        destination: "/docs/guides/skipping-tasks",
        permanent: true,
      },
      {
        source: "/reference/configuration",
        destination: "/docs/reference/configuration",
        permanent: true,
      },
      {
        source: "/reference/prune",
        destination: "/docs/reference/prune",
        permanent: true,
      },
      {
        source: "/repo/docs/ci/gitlabci",
        destination: "/docs/guides/ci-vendors/gitlab-ci",
        permanent: true,
      },
      {
        source: "/repo/docs/ci/travisci",
        destination: "/docs/guides/ci-vendors/travis-ci",
        permanent: true,
      },
      {
        source: "/docs/getting-started/existing-monorepos",
        destination: "/docs/getting-started/add-to-existing-repository",
        permanent: true,
      },
      {
        source: "/repo/docs/guides/ci-vendors/gitlabci",
        destination: "/docs/guides/ci-vendors/gitlab-ci",
        permanent: true,
      },
      {
        source: "/repo/docs/guides/ci-vendors/travisci",
        destination: "/docs/guides/ci-vendors/travis-ci",
        permanent: true,
      },
      {
        source: "/docs/guides/gitlab-ci",
        destination: "/docs/guides/ci-vendors/gitlab-ci",
        permanent: true,
      },
      {
        source: "/repo/docs/reference/command-line-reference/gen",
        destination: "/docs/reference/generate",
        permanent: true,
      },
      {
        source: "/roadmap",
        destination: "/blog",
        permanent: true,
      },
      {
        source: "/saml-sso-now-available",
        destination: "/blog/saml-sso-now-available",
        permanent: true,
      },
      {
        source: "/turbopack-benchmarks",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/why-turbopack",
        destination: "https://nextjs.org/docs/app/api-reference/turbopack",
        permanent: true,
      },
      {
        source: "/you-might-not-need-typescript-project-references",
        destination: "/blog/you-might-not-need-typescript-project-references",
        permanent: true,
      },
      {
        source: "/docs/core-concepts/monorepos/configuring-workspaces",
        destination: "/docs/reference/package-configurations",
        permanent: true,
      },
      {
        source: "/docs/handbook/linting/eslint",
        destination: "/docs/guides/tools/eslint",
        permanent: true,
      },
      {
        source: "/docs/reference/command-line-reference",
        destination: "/docs/reference",
        permanent: true,
      },
      {
        source: "/docs/guides/ci-vendors/gitlabci",
        destination: "/docs/guides/ci-vendors/gitlab-ci",
        permanent: true,
      },
      {
        source: "/docs/guides/ci-vendors/travisci",
        destination: "/docs/guides/ci-vendors/travis-ci",
        permanent: true,
      },
      {
        source: "/repo/docs/ci/gitlabci",
        destination: "/docs/guides/ci-vendors/gitlab-ci",
        permanent: true,
      },
      {
        source: "/repo/docs/ci/travisci",
        destination: "/docs/guides/ci-vendors/travis-ci",
        permanent: true,
      },
      {
        source: "/remote-cache",
        destination: "/docs/core-concepts/remote-caching",
        permanent: true,
      },
      {
        source: "/docs/troubleshooting",
        destination: "/docs/reference",
        permanent: true,
      },
      {
        source: "/docs/platform-environment-variables",
        destination:
          "/docs/crafting-your-repository/using-environment-variables#platform-environment-variables",
        permanent: true,
      },
      {
        source: "/docs/platform-environment-variables",
        destination:
          "/docs/crafting-your-repository/using-environment-variables#platform-environment-variables",
        permanent: true,
      },
      {
        source: "/docs/handbook",
        destination: "/docs/crafting-your-repository",
        permanent: true,
      },
      {
        source: "/docs/reference/codemods",
        destination: "/docs/reference/turbo-codemod",
        permanent: true,
      },
      {
        source: "/docs/getting-started/from-example",
        destination: "/docs/getting-started/examples",
        permanent: true,
      },
      {
        source: "/docs/getting-started/create-new",
        destination: "/docs/getting-started/installation",
        permanent: true,
      },
      {
        source: "/docs/reference/command-line-reference/run",
        destination: "/docs/reference/run",
        permanent: true,
      },
    ];
  },
};

// Required by Next.js, but we've extracted the config into a named export as well
export default withMDX(vercelToolbar(config));
