const path = require("path");

module.exports = {
  reactStrictMode: true,
  transpilePackages: ["@acme/ui"],
  output: "standalone",
  experimental: {
    outputFileTracingRoot: path.join(__dirname, "../../"),
  },
};
