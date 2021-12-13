const withNextra = require("nextra")({
  theme: "./nextra-theme-docs",
  themeConfig: "./theme.config.js",
  unstable_stork: false,
  unstable_staticImage: true,
});

module.exports = withNextra({
  reactStrictMode: true,
  experiments: {
    swcLoader: true,
    swcMinify: true,
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
        destination: "https://discord.gg/d6kXWZPWkW",
      },
      {
        source: "/docs/changelog",
        permanent: true,
        destination: "https://github.com/vercel/turborepo/releases",
      },
    ];
  },
});
