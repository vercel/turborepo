import { createMDX } from "fumadocs-mdx/next";
import type { NextConfig } from "next";

const withMDX = createMDX();

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

export default withMDX(config);
