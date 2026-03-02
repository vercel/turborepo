import type { MetadataRoute } from "next";

import { source } from "@/lib/geistdocs/source";

const protocol = process.env.NODE_ENV === "production" ? "https" : "http";
const baseUrl = `${protocol}://${process.env.NEXT_PUBLIC_VERCEL_PROJECT_PRODUCTION_URL}`;

export const revalidate = false;

export default function sitemap(): MetadataRoute.Sitemap {
  const url = (path: string): string => new URL(path, baseUrl).toString();

  const pages: MetadataRoute.Sitemap = [];

  for (const page of source.getPages()) {
    pages.push({
      changeFrequency: "weekly" as const,
      lastModified: page.data.lastModified
        ? new Date(page.data.lastModified)
        : undefined,
      priority: 0.5,
      url: url(page.url),
    });
  }

  return [
    {
      changeFrequency: "monthly",
      priority: 1,
      url: url("/"),
    },
    ...pages,
  ];
}
