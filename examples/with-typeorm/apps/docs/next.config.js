/** @type {import('next').NextConfig} */
module.exports = {
  transpilePackages: ["@repo/ui", "@repo/typeorm-service"],
  experimental: {
    serverComponentsExternalPackages: ["typeorm"],
  },
};
