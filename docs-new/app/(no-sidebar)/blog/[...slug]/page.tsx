import { getBlogPage, blogPageTree, getBlogPages } from "@/app/source";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";

export default async function Page({
  params,
}: {
  params: { slug?: string[] };
}) {
  const page = getBlogPage(params.slug);

  if (page == null) {
    notFound();
  }

  const MDX = page.data.exports.default;

  return (
    <article className="prose container w-full min-w-0 max-w-6xl mx-auto">
      <div className="my-4">
        <Link
          href="/blog"
          className="text-gray-500 dark:text-gray-400  hover:text-foreground transition-all"
        >
          ← Back to blog
        </Link>
      </div>
      <MDX />
    </article>
  );
}

export async function generateStaticParams() {
  return getBlogPages().map((page) => ({
    slug: page.slugs,
  }));
}

export function generateMetadata({ params }: { params: { slug?: string[] } }) {
  const page = getBlogPage(params.slug);

  if (page == null) notFound();

  return {
    title: page.data.title,
    description: page.data.description,
  } satisfies Metadata;
}
