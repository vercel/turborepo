import { blogPageTree, blogFiles } from "@/app/source";
import Link from "next/link";

const Page = () => {
  const posts = blogFiles.sort(
    (a, b) => new Date(b.data.exports.date) - new Date(a.data.exports.date)
  );

  return (
    <main className="flex flex-col gap-4 mt-8 mx-auto w-full min-w-0 max-w-6xl px-6 pt-4 md:px-12">
      <div className="w-screen-lg mx-auto w-full pt-4 pb-8 mb-16 border-b border-gray-400 border-opacity-20">
        <h1 className="text-center mt-2 mb-6 text-4xl font-bold tracking-tight leading-tight text-slate-900 dark:text-slate-100 lg:text-5xl">
          Blog
        </h1>
        <p className="text-center text-gray-500 dark:text-gray-400">
          The latest updates and releases from the Turbo team at Vercel.
        </p>
      </div>
      {posts.map((post) => (
        <Link
          href={post.url}
          className="mb-10 hover:underline block font-semibold mt-8 text-2xl"
        >
          <h2>{post.data.title}</h2>
          <p className="opacity-80 mt-2 font-normal text-base">
            {post.data.description}
          </p>

          <p className="opacity-80 mt-2 font-normal text-base">Read more →</p>
          <p className="opacity-50 mt-2 text-sm font-normal">
            {post.data.exports.date}
          </p>
          {/* <div key={post.file.path}>{post}</div> */}
        </Link>
      ))}
    </main>
  );
};

export default Page;
