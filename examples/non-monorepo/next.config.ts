import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  typescript: {
    ignoreBuildErrors: true,
  },
  experimental: {
    // This project runs TypeScript 7 (`tsc`) side-by-side with the TypeScript 6
    // API (`typescript`) so tooling like typescript-eslint keeps working while
    // TypeScript 7 ships no JavaScript API. Next.js must therefore load the
    // TypeScript 6 API instead of the TypeScript 7 CLI for its own checks.
    useTypeScriptCli: false,
  },
};

export default nextConfig;
