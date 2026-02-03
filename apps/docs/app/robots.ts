import type { MetadataRoute } from "next";

const PRODUCTION_DOMAIN = "turborepo.dev";

/**
 * Dynamic robots.txt generation.
 *
 * Only the production domain (turborepo.dev) is allowed to be indexed.
 * Subdomains (e.g., v1.turborepo.dev) and preview deployments are blocked.
 */
export default function robots(): MetadataRoute.Robots {
  // VERCEL_PROJECT_PRODUCTION_URL is the production domain assigned to the project
  // For the main site, this will be "turborepo.dev"
  // For subdomains, this will be "v1.turborepo.dev", etc.
  const productionUrl = process.env.VERCEL_PROJECT_PRODUCTION_URL ?? "";

  if (productionUrl !== PRODUCTION_DOMAIN) {
    return {
      rules: {
        userAgent: "*",
        disallow: "/"
      }
    };
  }

  return {
    rules: {
      userAgent: "*",
      allow: "/"
    },
    sitemap: `https://${PRODUCTION_DOMAIN}/sitemap.xml`
  };
}
