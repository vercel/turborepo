import { promises as fs, statSync } from "node:fs";
import path from "node:path";
import RSS from "rss";
import matter from "gray-matter";

interface FrontMatter {
  data: {
    date: string;
    title: string;
    description: string;
    ogImage: string;
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
  // @ts-expect-error -- Package lacks typings
  // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-call -- Package lacks types
  const feed = new RSS({
    title: "Turborepo Blog",
    description: "Turborepo news, updates, and announcements.",
    site_url: "https://turborepo.com",
    feed_url: "https://turborepo.com/feed.xml",
    image_url: "https://turborepo.com/api/og",
  });

  const posts = await fs.readdir(path.join(__dirname, "..", "content", "blog"));

  const promises = posts.map(async (post: string) => {
    if (post.startsWith("index.") || post.startsWith("_meta.json")) return;
    const file = await fs.readFile(
      path.join(__dirname, "..", "content", "blog", post)
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
    // get the og image size
    const stat = statSync(
      path.join(__dirname, "..", "public", frontmatter.data.ogImage)
    );
    // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access -- Package lacks types
    feed.item({
      title: frontmatter.data.title,
      url: `https://turborepo.com/blog/${frontmatter.slug}`, // intentionally including slash here
      date: frontmatter.data.date,
      description: frontmatter.data.description,
      enclosure: {
        url: `https://turborepo.com${frontmatter.data.ogImage}`, // intentionally omitting slash here
        type: "image/png",
        size: stat.size,
      },
    });
  }

  // eslint-disable-next-line @typescript-eslint/no-unsafe-call, @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-argument -- Package lacks types
  await fs.writeFile("./public/feed.xml", feed.xml({ indent: true }));
}

void generate();
