const loadConfig = require("next/dist/server/config").default;
const { PHASE_DEVELOPMENT_SERVER } = require("next/dist/shared/lib/constants");

module.exports.execute = async function execute(path) {
  const nextConfig = await loadConfig(PHASE_DEVELOPMENT_SERVER, path);
  nextConfig.rewrites = await nextConfig.rewrites?.();
  nextConfig.redirects = await nextConfig.redirects?.();
  return nextConfig;
};
