// next.config.js
const withNextra = require("nextra")("./nextra-theme", "./theme.config.js");

module.exports = withNextra({
  basePath: "/docs",
  experimental: {
    turboMode: true,
    esmExternals: true,
  },
  async redirects() {
    return [
      {
        source: "/usage",
        destination: "/reference/command-line-reference",
        permanent: true,
      },
    ];
  },
});
