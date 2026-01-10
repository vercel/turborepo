import { source, blog, openapiPages } from "@/lib/geistdocs/source";

/**
 * Static routes that are not part of fumadocs loaders.
 * These are standalone pages in the app directory.
 */
const STATIC_ROUTES = ["/", "/blog", "/showcase", "/governance", "/terms"];

/**
 * Collect all page URLs from fumadocs loaders and static routes.
 *
 * This ensures the sitemap is always exhaustive by:
 * 1. Including known static routes
 * 2. Getting all dynamic routes from fumadocs loaders (docs, blog, etc.)
 *
 * The results are deduplicated to handle any overlap.
 */
export function getAllPageUrls(): Array<string> {
  const urlSet = new Set<string>();

  // 1. Add known static routes
  for (const route of STATIC_ROUTES) {
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
