import { Breadcrumb } from "./_components/Breadcrumb";
import { getBlogPage, blogPageTree, getBlogPages } from "@/app/source";
import type { Metadata } from "next";
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

  console.log(blogPageTree);

  return (
    <article className="prose container">
      <Breadcrumb tree={blogPageTree} />
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
