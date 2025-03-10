const { promises: fs, statSync } = require('node:fs');
const path = require('node:path');
const RSS = require('rss');
const matter = require('gray-matter');

function dateSortDesc(a, b) {
  const date1 = new Date(a.data.date);
  const date2 = new Date(b.data.date);
  if (date1 > date2) return -1;
  if (date1 < date2) return 1;
  return 0;
}

async function generate() {
  const feed = new RSS({
    title: 'Turbo Blog',
    description: 'Turbo news, updates, and announcements.',
    site_url: 'https://turbo.build',
    feed_url: 'https://turbo.build/feed.xml',
    image_url: 'https://turbo.build/api/og',
  });

  const posts = await fs.readdir(path.join(__dirname, '..', 'content', 'blog'));

  const promises = posts.map(async (post) => {
    if (post.startsWith('index.') || post.startsWith('_meta.json')) return;
    const file = await fs.readFile(
      path.join(__dirname, '..', 'content', 'blog', post),
    );
    const frontmatter = matter(file);
    if (frontmatter.data.href) return;
    return { ...frontmatter, slug: post.replace('.mdx', '') };
  });

  const results = await Promise.all(promises);
  const sortedData = results.filter(Boolean); // Remove null values

  // sort by date
  sortedData.sort(dateSortDesc);

  for (const frontmatter of sortedData) {
    // get the og image size
    const stat = statSync(
      path.join(__dirname, '..', 'public', frontmatter.data.ogImage),
    );
    feed.item({
      title: frontmatter.data.title,
      url: `https://turbo.build/blog/${frontmatter.slug}`, // intentionally including slash here
      date: frontmatter.data.date,
      description: frontmatter.data.description,
      enclosure: {
        url: `https://turbo.build${frontmatter.data.ogImage}`, // intentionally omitting slash here
        type: 'image/png',
        size: stat.size,
      },
    });
  }

  await fs.writeFile('./public/feed.xml', feed.xml({ indent: true }));
}

generate();
