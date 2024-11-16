/** @type {import('next').NextConfig} */
module.exports = {
  transpilePackages: ["@repo/ui"],
  experimental: {
    serverComponentsExternalPackages: ["typeorm","@medusajs/medusa"],
  },
};
