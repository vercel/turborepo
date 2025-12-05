import type { MetadataRoute } from "next";
import { repoDocsPages, blog, extraPages } from "#app/source.ts";
import { openapiPages } from "#app/(openapi)/docs/openapi/source.ts";
import { loadState, SITEMAP_CONFIG } from "#lib/sitemap/index.ts";

/**
 * Collect all page URLs from fumadocs loaders
 */
function getAllPageUrls(): Array<string> {
  const urls: Array<string> = [];

  // Add homepage
  urls.push("/");

  // Docs pages
  for (const page of repoDocsPages.getPages()) {
    urls.push(page.url);
  }

  // Blog pages (exclude external blog posts)
  for (const page of blog.getPages()) {
    urls.push(page.url);
  }

  // Extra pages (governance, terms, etc.)
  for (const page of extraPages.getPages()) {
    urls.push(page.url);
  }

  // OpenAPI pages
  for (const page of openapiPages.getPages()) {
    urls.push(page.url);
  }

  // Add showcase page
  urls.push("/showcase");

  return urls;
}

export const dynamic = "force-dynamic";
export const revalidate = 3600; // Revalidate every hour

// eslint-disable-next-line import/no-default-export -- Required by Next.js sitemap convention
export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  // Load state from Redis
  const state = await loadState();

  // Get all page URLs
  const pageUrls = getAllPageUrls();

  // Build sitemap entries
  const entries: MetadataRoute.Sitemap = pageUrls.map((url) => {
    const pageState = state?.pages[url];

    return {
      url: `${SITEMAP_CONFIG.baseUrl}${url}`,
      lastModified: pageState?.lastmod
        ? new Date(pageState.lastmod)
        : new Date(),
    };
  });

  return entries;
}
