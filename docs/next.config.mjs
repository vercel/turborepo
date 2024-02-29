import createMDX from "fumadocs-mdx/config";
import { withVercelToolbar } from "@vercel/toolbar/plugins/next";

const withMDX = createMDX();
const vercelToolbar = withVercelToolbar();

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  typescript: {
    ignoreBuildErrors: true,
  },
  eslint: {
    ignoreDuringBuilds: true,
  },
  experimental: {
    mdxRs: true,
    useLightningcss: true,
  },
};

export default withMDX(vercelToolbar(config));
// export default withMDX(config);
