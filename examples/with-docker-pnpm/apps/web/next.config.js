const path = require("path");

module.exports = {
  reactStrictMode: true,
  // transpilePackages: ["ui"],
  output: "standalone",
  experimental: {
    appDir: true,
    outputFileTracingRoot: path.join(__dirname, "../../"),
  },
};
