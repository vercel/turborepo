import loadConfig from "next/dist/server/config";
// import getBaseWebpackConfig from "next/dist/build/webpack-config";
import { PHASE_DEVELOPMENT_SERVER } from "next/dist/shared/lib/constants";

const loadNextConfig = async () => {
  const nextConfig = await loadConfig(PHASE_DEVELOPMENT_SERVER, process.cwd());
  console.log("CONFIG", nextConfig.webpack);
  nextConfig.rewrites = await nextConfig.rewrites?.();
  nextConfig.redirects = await nextConfig.redirects?.();
  return nextConfig;
};

export { loadNextConfig as default };
