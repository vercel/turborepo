import type { Metadata } from "next/types";
import Link from "next/link";
import { blog, externalBlog } from "@/lib/geistdocs/source";
import { createMetadata } from "@/lib/create-metadata";
import { getLocalizedPath } from "@/lib/geistdocs/public-path";
import { getRootLang } from "@/lib/geistdocs/root-params";
import { absoluteUrl } from "@/lib/geistdocs/site-url";

export async function generateMetadata({
  params
}: PageProps<"/[lang]/blog">): Promise<Metadata> {
  const { lang } = await params;
  const baseMetadata = createMetadata({
    title: "Blog",
    description: "Get the latest news and updates from the Turboverse.",
    canonicalPath: getLocalizedPath(lang, "/blog")
  });

  return {
    ...baseMetadata,
    alternates: {
      ...baseMetadata.alternates,
      types: {
        "application/rss+xml": absoluteUrl("/feed.xml")
      }
    }
  };
}

async function Page() {
  const lang = await getRootLang();
  const posts = [...blog.getPages(), ...externalBlog.getPages()].sort(
    (a, b) => {
      return Number(new Date(b.data.date)) - Number(new Date(a.data.date));
    }
  );

  return (
    <div className="mx-auto mt-8 flex w-full min-w-0 max-w-6xl flex-col gap-4 px-6 pt-14 md:px-12">
      <div className="w-screen-lg mx-auto mb-16 w-full border-b border-gray-100/10 border-opacity-20 pb-8 pt-4">
        <h1 className="mb-6 mt-2 text-center text-heading-48 text-slate-900 dark:text-slate-100 lg:text-heading-56">
          Blog
        </h1>
        <p className="text-center text-copy-20 text-gray-900">
          The latest updates and releases from the Turborepo team
        </p>
      </div>
      {posts.map((post) => {
        if ("isExternal" in post.data) {
          return (
            <Link
              className="mb-10 block hover:underline"
              href={post.data.href}
              key={post.data.title}
              target="_blank"
            >
              <h2 className="text-heading-32">{post.data.title}</h2>
              <p className="mt-2 text-base font-normal opacity-80">
                {post.data.description}
              </p>

              <p className="mt-2 text-base font-normal opacity-80">
                Read more →
              </p>
              <p className="mt-2 text-sm font-normal opacity-50">
                {post.data.date}
              </p>
            </Link>
          );
        }

        return (
          <Link
            className="mb-10 block hover:underline"
            href={getLocalizedPath(lang, `/blog/${post.slugs.join("/")}`)}
            key={post.data.title}
            prefetch
            target={undefined}
          >
            <h2 className="text-heading-32">{post.data.title}</h2>
            <p className="mt-2 text-base font-normal opacity-80">
              {post.data.description}
            </p>

            <p className="mt-2 text-base font-normal opacity-80">Read more →</p>
            <p className="mt-2 text-sm font-normal opacity-50">
              {post.data.date}
            </p>
          </Link>
        );
      })}
    </div>
  );
}

export default Page;
