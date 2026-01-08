import { withVercelToolbar } from "@vercel/toolbar/plugins/next";
import { createMDX } from "fumadocs-mdx/next";
import type { NextConfig } from "next";

const withMDX = createMDX();
const vercelToolbar = withVercelToolbar();

const config: NextConfig = {
  experimental: {
    turbopackFileSystemCacheForDev: true
  },
  typescript: {
    ignoreBuildErrors: true
  },

  // biome-ignore lint/suspicious/useAwait: rewrite is async
  async rewrites() {
    return [
      {
        source: "/docs/:path*.mdx",
        destination: "/llms.mdx/:path*"
      },
      {
        source: "/docs/:path*.md",
        destination: "/llms.mdx/:path*"
      }
    ];
  },

  // biome-ignore lint/suspicious/useAwait: redirect is async
  async redirects() {
    return [
      // OpenAPI redirects (until we have more content)
      {
        source: "/docs/openapi",
        destination: "/docs/openapi/artifacts/artifact-exists",
        permanent: false
      },
      {
        source: "/docs/openapi/artifacts",
        destination: "/docs/openapi/artifacts/artifact-exists",
        permanent: false
      }
    ];
  },

  images: {
    formats: ["image/avif", "image/webp"],
    remotePatterns: [
      {
        protocol: "https",
        hostname: "placehold.co"
      },
      {
        protocol: "https",
        hostname: "ufa25dqjajkmio0q.public.blob.vercel-storage.com"
      }
    ]
  }
};

export default withMDX(vercelToolbar(config));
