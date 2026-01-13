import type { MetadataRoute } from "next";

/**
 * Check if the host is a subdomain (has more than 2 parts, e.g., v1.example.com)
 */
function isSubdomain(host: string): boolean {
  const parts = host.split(".");
  return parts.length > 2;
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
