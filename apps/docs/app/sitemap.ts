import type { MetadataRoute } from "next";
import { cacheLife } from "next/cache";
import { loadState, getAllPageUrls, SITEMAP_CONFIG } from "../lib/sitemap";

// eslint-disable-next-line import/no-default-export -- Required by Next.js sitemap convention
export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  "use cache";
  cacheLife("hours");

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
