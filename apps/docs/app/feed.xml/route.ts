import { Feed } from "feed";
import { cacheLife } from "next/cache";
import { blog } from "@/lib/geistdocs/source";
import { absoluteUrl, siteUrl } from "@/lib/geistdocs/site-url";
import { createSignedOgUrl } from "@/lib/og/sign";

const BASE_URL = siteUrl.origin;

/**
 * Escapes `&` so URLs render as valid XML attribute values.
 * The `feed` library escapes `&` in text nodes but not in the
 * `enclosure` URL attribute, and `xml-js` leaves attribute values raw,
 * so signed OG image URLs with query params break feed.xml.
 */
const escapeXmlAmpersand = (url: string): string => url.replace(/&/g, "&amp;");

const getFeed = async () => {
  "use cache";
  cacheLife("max");

  const feed = new Feed({
    title: "Turborepo Blog",
    description: "Turborepo news, updates, and announcements.",
    id: BASE_URL,
    link: BASE_URL,
    image: absoluteUrl(createSignedOgUrl("", "Turborepo")),
    favicon: absoluteUrl("/favicon.ico"),
    copyright: `All rights reserved ${new Date().getFullYear()}, Vercel Inc.`,
    feedLinks: {
      rss2: `${BASE_URL}/feed.xml`
    }
  });

  const posts = blog.getPages().sort((a, b) => {
    return Number(new Date(b.data.date)) - Number(new Date(a.data.date));
  });

  for (const post of posts) {
    const slug = post.slugs.join("/");

    const imageUrl = absoluteUrl(createSignedOgUrl(post.data.title, "Blog"));

    feed.addItem({
      title: post.data.title,
      id: `${BASE_URL}/blog/${slug}`,
      link: `${BASE_URL}/blog/${slug}`,
      date: new Date(post.data.date),
      description: post.data.description,
      enclosure: {
        url: escapeXmlAmpersand(imageUrl),
        length: 0,
        type: "image/png"
      }
    });
  }

  return feed.rss2();
};

export const GET = async () => {
  return new Response(await getFeed(), {
    headers: {
      "Content-Type": "application/rss+xml"
    }
  });
};
