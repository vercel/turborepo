import type { MetadataRoute } from "next";
import { loadState, getAllPageUrls, SITEMAP_CONFIG } from "../lib/sitemap";

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
        : new Date()
    };
  });

  return entries;
}
