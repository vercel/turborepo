import type { Metadata } from "next";
import { DocsPage, DocsBody } from "fumadocs-ui/page";
import * as Base from "fumadocs-core/toc";
import { notFound } from "next/navigation";
import { getPage, getPages } from "@/app/source";
import { RemoteCacheCounter } from "@/components/RemoteCacheCounter";

export default function Page({ params }: { params: { slug?: string[] } }) {
  const page = getPage(params.slug);

  if (!page) {
    notFound();
  }

  const Mdx = page.data.exports.default;

  return (
    <DocsPage
      toc={page.data.exports.toc}
      tableOfContent={{
        header: <RemoteCacheCounter />,
      }}
    >
      <DocsBody>
        <h1 className="text-left">{page.data.title}</h1>
        <Mdx />
      </DocsBody>
    </DocsPage>
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
