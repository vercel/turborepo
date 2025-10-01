import Link from "next/link";
import type { Metadata } from "next/types";
import { blog, externalBlog } from "#app/source.ts";
import { createMetadata } from "#lib/create-metadata.ts";
import { FaviconHandler } from "#app/_components/favicon-handler.tsx";

export function generateMetadata(): Metadata {
  const rawMetadata = createMetadata({
    title: "Blog",
    description: "Get the latest news and updates from the Turboverse.",
    canonicalPath: "/blog",
  });

  return {
    ...rawMetadata,
    alternates: {
      ...rawMetadata.alternates,
      types: {
        ...rawMetadata.alternates?.types,
        "application/rss+xml": `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}/feed.xml`,
      },
    },
  };
}

function Page(): JSX.Element {
  const posts = [...blog.getPages(), ...externalBlog.getPages()].sort(
    (a, b) => {
      return Number(new Date(b.data.date)) - Number(new Date(a.data.date));
    }
  );

  return (
    <main className="mx-auto mt-8 flex w-full min-w-0 max-w-6xl flex-col gap-4 px-6 pt-14 md:px-12">
      <FaviconHandler />
      <div className="w-screen-lg mx-auto mb-16 w-full border-b border-gray-400 border-opacity-20 pb-8 pt-4">
        <h1 className="mb-6 mt-2 text-center text-4xl font-bold leading-tight tracking-tight text-slate-900 dark:text-slate-100 lg:text-5xl">
          Blog
        </h1>
        <p className="text-center text-gray-900 dark:text-gray-900">
          The latest updates and releases from the Turborepo team at Vercel.
        </p>
      </div>
      {posts.map((post) => {
        if ("isExternal" in post.data) {
          return (
            <Link
              className="mb-10 block text-2xl font-semibold hover:underline"
              href={post.data.href}
              key={post.data.title}
              target="_blank"
            >
              <h2>{post.data.title}</h2>
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
            className="mb-10 block text-2xl font-semibold hover:underline"
            href={`/blog/${post.slugs.join("/")}`}
            key={post.data.title}
            target={undefined}
          >
            <h2>{post.data.title}</h2>
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
    </main>
  );
}

export default Page;
