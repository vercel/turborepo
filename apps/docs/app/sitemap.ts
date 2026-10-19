import { getPublicPath } from "@vercel/geistdocs/config";
import { isPageVisibleForSurface } from "@vercel/geistdocs/page-visibility";
import type { MetadataRoute } from "next";
import { cacheLife } from "next/cache";
import { config } from "@/lib/geistdocs/config";
import { absoluteUrl } from "@/lib/geistdocs/site-url";
import {
  blog,
  extraPages,
  openapiGeistdocsSource,
  source
} from "@/lib/geistdocs/source";

interface SitemapPage {
  data: unknown;
  url: string;
}

const url = (path: string) => absoluteUrl(getPublicPath(path, config.basePath));

const toEntry = (page: SitemapPage): MetadataRoute.Sitemap[number] => {
  const { lastModified } = page.data as { lastModified?: Date };

  return {
    changeFrequency: "weekly",
    lastModified: lastModified ? new Date(lastModified) : undefined,
    priority: 0.5,
    url: url(page.url)
  };
};

// eslint-disable-next-line import/no-default-export -- Required by Next.js sitemap convention
export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  "use cache";
  cacheLife("max");

  // Translated locales fall back to English content, so only the default
  // language is listed to keep duplicate pages out of the sitemap.
  const pages: Array<SitemapPage> = [
    ...source.getPages(config.defaultLanguage),
    ...openapiGeistdocsSource.source.getPages(config.defaultLanguage),
    ...blog.getPages(),
    ...extraPages.getPages()
  ];

  return [
    { changeFrequency: "monthly", priority: 1, url: url("/") },
    { changeFrequency: "weekly", priority: 0.5, url: url("/blog") },
    { changeFrequency: "monthly", priority: 0.5, url: url("/showcase") },
    ...pages
      .filter((page) => isPageVisibleForSurface(page, "sitemap"))
      .map(toEntry)
  ];
}
