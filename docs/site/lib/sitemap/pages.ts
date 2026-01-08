import { source, blog, openapiPages } from "@/lib/geistdocs/source";
import { discoverStaticRoutes, getAppDirectory } from "./route-discovery";

/**
 * Collect all page URLs from both automatic route discovery and fumadocs loaders.
 *
 * This ensures the sitemap is always exhaustive by:
 * 1. Scanning the app directory for all static page.tsx files
 * 2. Getting all dynamic routes from fumadocs loaders (docs, blog, etc.)
 *
 * The results are deduplicated to handle any overlap.
 */
export function getAllPageUrls(): Array<string> {
  const urlSet = new Set<string>();

  // 1. Discover static routes from app directory
  // This catches standalone pages like /blog, /showcase, etc.
  const staticRoutes = discoverStaticRoutes(getAppDirectory());
  for (const route of staticRoutes) {
    urlSet.add(route);
  }

  // 2. Add dynamic routes from fumadocs loaders
  // These handle content-driven pages with [...slug] patterns

  // Docs pages
  for (const page of source.getPages()) {
    urlSet.add(page.url);
  }

  // Blog pages (exclude external blog posts - they link off-site)
  for (const page of blog.getPages()) {
    urlSet.add(page.url);
  }

  // OpenAPI pages
  for (const page of openapiPages.getPages()) {
    urlSet.add(page.url);
  }

  return [...urlSet].sort();
}
