import { getBlogPage, blogPageTree, getBlogPages } from "@/app/source";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { ArrowLeftIcon } from "@heroicons/react/outline";

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
    <article className="prose max-w-prose mx-auto mb-10">
      <div className="my-4">
        <Link
          href="/blog"
          className="text-sm flex flex-row gap-2 text-gray-500 dark:text-gray-400  hover:text-foreground transition-all"
        >
          <ArrowLeftIcon width=".75rem" />
          Back to blog
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
