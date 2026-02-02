import Link from "next/link";
import { blog, externalBlog } from "@/lib/geistdocs/source";

export default function NotFound() {
  const posts = [...blog.getPages(), ...externalBlog.getPages()]
    .sort((a, b) => {
      return Number(new Date(b.data.date)) - Number(new Date(a.data.date));
    })
    .slice(0, 3);

  return (
    <main className="mx-auto mt-8 flex w-full min-w-0 max-w-6xl flex-col gap-4 px-6 pt-14 md:px-12">
      <div className="w-screen-lg mx-auto mb-16 w-full border-b border-gray-400 border-opacity-20 pb-8 pt-4">
        <h1 className="mb-6 mt-2 text-center text-4xl font-bold leading-tight tracking-tight text-slate-900 dark:text-slate-100 lg:text-5xl">
          Blog post not found
        </h1>
        <p className="text-center text-gray-600 dark:text-gray-400">
          The blog post you are looking for does not exist. Here are some recent
          posts:
        </p>
      </div>
      <div className="flex flex-col gap-12 mx-auto">
        {posts.map((post) => {
          if ("isExternal" in post.data) {
            return (
              <Link
                className="block text-2xl font-semibold hover:underline"
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
              className="block text-2xl font-semibold hover:underline"
              href={`/blog/${post.slugs.join("/")}`}
              key={post.data.title}
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
        })}
        <p className="prose dark:prose-invert">
          <Link href="/blog">Find more posts</Link> or{" "}
          <Link href="/">head back to home</Link>.
        </p>
      </div>
    </main>
  );
}
