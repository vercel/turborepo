import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Feed } from "feed";
import matter from "gray-matter";

interface FrontMatter {
  data: {
    date: string;
    title: string;
    description: string;
    ogImage?: string;
    href?: string;
  };
  content: string;
  slug?: string;
}

function dateSortDesc(a: FrontMatter, b: FrontMatter): number {
  const date1 = new Date(a.data.date);
  const date2 = new Date(b.data.date);
  if (date1 > date2) return -1;
  if (date1 < date2) return 1;
  return 0;
}

async function generate(): Promise<void> {
  const feed = new Feed({
    title: "Turborepo Blog",
    description: "Turborepo news, updates, and announcements.",
    id: "https://turborepo.com",
    link: "https://turborepo.com",
    image: "https://turborepo.com/api/og",
    favicon: "https://turborepo.com/favicon.ico",
    copyright: `All rights reserved ${new Date().getFullYear()}, Vercel Inc.`,
    feedLinks: {
      rss2: "https://turborepo.com/feed.xml"
    }
  });

  const currentDir = path.dirname(fileURLToPath(import.meta.url));

  const posts = await fs.readdir(
    path.join(currentDir, "..", "content", "blog")
  );

  const promises = posts.map(async (post: string) => {
    if (post.startsWith("index.") || post.startsWith("_meta.json")) return;
    const file = await fs.readFile(
      path.join(currentDir, "..", "content", "blog", post)
    );
    const { data, content } = matter(file);
    if (data.href) return;
    return { data, content, slug: post.replace(".mdx", "") } as FrontMatter;
  });

  const results = await Promise.all(promises);
  const sortedData = results.filter(
    (item): item is FrontMatter & { slug: string } => Boolean(item)
  );

  // sort by date
  sortedData.sort(dateSortDesc);

  for (const frontmatter of sortedData) {
    const createOgUrl = () => {
      const groups = /^turbo-(?<major>\d+)-(?<minor>\d+)(?:-\d+)*$/.exec(
        frontmatter.slug
      );
      if (groups) {
        const { major, minor } = groups.groups as {
          major: string;
          minor: string;
        };
        return `/api/og/blog?version=${encodeURIComponent(
          `${major}.${minor}`
        )}`;
      }

      return "▲";
    };

    feed.addItem({
      title: frontmatter.data.title,
      id: `https://turborepo.com/blog/${frontmatter.slug}`,
      link: `https://turborepo.com/blog/${frontmatter.slug}`,
      date: new Date(frontmatter.data.date),
      description: frontmatter.data.description,
      image: `https://turborepo.com${frontmatter.data.ogImage ?? createOgUrl()}`
    });
  }

  await fs.writeFile("./public/feed.xml", feed.rss2());
}

void generate();
