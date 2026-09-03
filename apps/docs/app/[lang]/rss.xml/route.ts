import { Feed } from "feed";
import { cacheLife } from "next/cache";
import type { NextRequest } from "next/server";
import { title } from "@/geistdocs";
import { absoluteUrl } from "@/lib/geistdocs/site-url";
import { source } from "@/lib/geistdocs/source";

const baseUrl = absoluteUrl("/");

const getFeed = async (lang: string) => {
  "use cache";
  cacheLife("max");

  const feed = new Feed({
    title,
    id: baseUrl,
    link: baseUrl,
    language: lang,
    copyright: `All rights reserved ${new Date().getFullYear()}, Vercel`
  });

  for (const page of source.getPages(lang)) {
    // The geistdocs source bundle erases the collection's frontmatter typing;
    // narrow structurally to the fields this feed needs.
    const data = page.data as {
      description?: string;
      lastModified?: Date;
      title?: string;
    };

    feed.addItem({
      id: page.url,
      title: data.title ?? page.url,
      description: data.description,
      link: absoluteUrl(page.url),
      date: new Date(data.lastModified ?? new Date()),
      author: [
        {
          name: "Vercel"
        }
      ]
    });
  }

  return feed.rss2();
};

export const GET = async (
  _req: NextRequest,
  { params }: RouteContext<"/[lang]/rss.xml">
) => {
  const { lang } = await params;
  const rss = await getFeed(lang);

  return new Response(rss, {
    headers: {
      "Content-Type": "application/rss+xml"
    }
  });
};
