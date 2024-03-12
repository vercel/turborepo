import { notFound } from "next/navigation";
import { getPage, getPages } from "@/app/source";
import { Metadata } from "next";

export default function Page({ params }: { params: { slug?: string[] } }) {
  const page = getPage(params.slug);

  if (!page) {
    notFound();
  }

  const Mdx = page.data.exports.default;

  return (
    <>
      <h1 className="text-left">{page.data.title}</h1>
      <Mdx />
    </>
  );
}

export function generateStaticParams() {
  return getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export function generateMetadata({ params }: { params: { slug?: string[] } }) {
  const page = getPage(params.slug);

  if (!page) notFound();

  return {
    title: page.data.title,
    description: page.data.description,
  } satisfies Metadata;
}
