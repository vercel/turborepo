import { Feed } from "feed";
import { blog } from "@/lib/geistdocs/source";

const BASE_URL = "https://turborepo.dev";

export const revalidate = false;

export const GET = async () => {
  const feed = new Feed({
    title: "Turborepo Blog",
    description: "Turborepo news, updates, and announcements.",
    id: BASE_URL,
    link: BASE_URL,
    image: `${BASE_URL}/api/og`,
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

    const createOgUrl = () => {
      const groups = /^turbo-(?<major>\d+)-(?<minor>\d+)(?:-\d+)*$/.exec(slug);
      if (groups?.groups) {
        const { major, minor } = groups.groups;
        return `/api/og/blog?version=${encodeURIComponent(`${major}.${minor}`)}`;
      }
      return undefined;
    };

    const ogUrl = createOgUrl();
    const imageUrl = post.data.ogImage
      ? `${BASE_URL}${post.data.ogImage}`
      : ogUrl
        ? `${BASE_URL}${ogUrl}`
        : undefined;

    feed.addItem({
      title: post.data.title,
      id: `${BASE_URL}/blog/${slug}`,
      link: `${BASE_URL}/blog/${slug}`,
      date: new Date(post.data.date),
      description: post.data.description,
      ...(imageUrl && {
        enclosure: { url: imageUrl, length: 0, type: "image/png" }
      })
    });
  }

  return new Response(feed.rss2(), {
    headers: {
      "Content-Type": "application/rss+xml"
    }
  });
};
