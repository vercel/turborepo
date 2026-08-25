import { Feed } from "feed";
import { blog } from "@/lib/geistdocs/source";
import { createSignedOgUrl } from "@/lib/og/sign";

const BASE_URL = "https://turborepo.dev";

/**
 * Escapes `&` so URLs render as valid XML attribute values.
 * The `feed` library escapes `&` in text nodes but not in the
 * `enclosure` URL attribute, and `xml-js` leaves attribute values raw,
 * so signed OG image URLs with query params break feed.xml.
 */
const escapeXmlAmpersand = (url: string): string =>
  url.replace(/&/g, "&amp;");

export const revalidate = false;

export const GET = async () => {
  const feed = new Feed({
    title: "Turborepo Blog",
    description: "Turborepo news, updates, and announcements.",
    id: BASE_URL,
    link: BASE_URL,
    image: `${BASE_URL}${createSignedOgUrl("", "Turborepo")}`,
    favicon: `${BASE_URL}/favicon.ico`,
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

    const imageUrl = `${BASE_URL}${createSignedOgUrl(post.data.title, "Blog")}`;

    feed.addItem({
      title: post.data.title,
      id: `${BASE_URL}/blog/${slug}`,
      link: `${BASE_URL}/blog/${slug}`,
      date: new Date(post.data.date),
      description: post.data.description,
      enclosure: { url: escapeXmlAmpersand(imageUrl), length: 0, type: "image/png" }
    });
  }

  return new Response(feed.rss2(), {
    headers: {
      "Content-Type": "application/rss+xml"
    }
  });
};
