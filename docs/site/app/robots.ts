import type { MetadataRoute } from "next";

/**
 * Check if the host is a subdomain of turborepo.com (e.g., v1.turborepo.com)
 */
function isSubdomain(host: string): boolean {
  return host.endsWith(".turborepo.com");
}

/**
 * Dynamic robots.txt generation.
 *
 * All subdomains are blocked from search engine indexing.
 */
export default function robots(): MetadataRoute.Robots {
  const host = process.env.VERCEL_URL ?? "";

  if (isSubdomain(host)) {
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
    }
  };
}
