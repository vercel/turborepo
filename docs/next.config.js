const withNextra = require("nextra")({
  theme: "nextra-theme-docs",
  themeConfig: "./theme.config.js",
  unstable_flexsearch: true,
  unstable_staticImage: true,
});

module.exports = withNextra({
  reactStrictMode: true,
  experimental: {
    legacyBrowsers: false,
    images: { allowFutureImage: true },
  },
  async redirects() {
    return [
      {
        source: "/usage",
        destination: "/reference/command-line-reference",
        permanent: true,
      },
      {
        source: "/docs/pipelines",
        destination: "/docs/running-tasks",
        permanent: true,
      },
      {
        source: "/discord{/}?",
        permanent: true,
        destination: "https://discord.gg/sSzyjxvbf5",
      },
      {
        source: "/docs/changelog",
        permanent: true,
        destination: "https://github.com/vercel/turborepo/releases",
      },
      {
        source: "/docs/guides/complimentary-tools",
        permanent: true,
        destination: "/docs/guides/monorepo-tools",
      },
      {
        source: "/docs/guides/continuous-integration",
        permanent: true,
        destination: "/docs/ci",
      },
      {
        source: "/docs/features/:path*",
        permanent: true,
        destination: "/docs/core-concepts/:path*",
      },
    ];
  },
});
