const withNextra = require("nextra")({
  theme: "./nextra-theme-docs",
  themeConfig: "./theme.config.js",
  unstable_contentDump: true,
  unstable_staticImage: true,
});

module.exports = withNextra({
  // reactStrictMode: true,
  experiments: {
    esmExternals: true,
  },
  async redirects() {
    return [
      {
        source: "/usage",
        destination: "/reference/command-line-reference",
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
    ];
  },
});
