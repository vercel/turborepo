import Link from "next/link";
import { blog, externalBlog } from "#app/source.ts";
import { NotFoundTemplate } from "../../_components/not-found-template";

export default function NotFound(): JSX.Element {
  const posts = [...blog.getPages(), ...externalBlog.getPages()]
    .sort((a, b) => {
      return Number(new Date(b.data.date)) - Number(new Date(a.data.date));
    })
    .slice(0, 3);

  return (
    <NotFoundTemplate
      content={
        <div className="flex flex-col gap-12 mx-auto">
          <p className="text-center text-gray-900 dark:text-gray-900">
            The latest updates and releases from the Turborepo team at Vercel.
          </p>
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
          <p className="prose">
            <Link href="/blog">Find more posts</Link> or{" "}
            <Link href="/">head back to home</Link>.
          </p>
        </div>
      }
    />
  );
}
