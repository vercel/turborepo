import Link from "next/link";
import { notFound } from "next/navigation";
import { ArrowLeft } from "lucide-react";
import type { Metadata } from "next";
import { blog } from "@/lib/geistdocs/source";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { createMetadata } from "@/lib/create-metadata";
import { getLocalizedPath } from "@/lib/geistdocs/public-path";
import { createSignedOgUrl } from "@/lib/og/sign";

export function generateStaticParams(): Array<{ slug: Array<string> }> {
  return blog.getPages().map((page) => ({
    slug: page.slugs
  }));
}

export async function generateMetadata({
  params
}: PageProps<"/[lang]/blog/[...slug]">): Promise<Metadata> {
  const { lang, slug } = await params;
  const page = blog.getPage(slug);

  if (!page) notFound();

  const metadata = createMetadata({
    title: page.data.title,
    description: page.data.description,
    canonicalPath: getLocalizedPath(lang, `/blog/${slug.join("/")}`)
  });

  return {
    ...metadata,
    openGraph: {
      ...metadata.openGraph,
      images: [{ url: createSignedOgUrl(page.data.title, "Blog") }]
    }
  };
}

export default async function Page({
  params
}: PageProps<"/[lang]/blog/[...slug]">) {
  const { lang, slug } = await params;
  const page = blog.getPage(slug);

  if (!page) notFound();

  const Mdx = page.data.body;

  return (
    <article className="prose dark:prose-invert mx-auto mb-10 w-full min-w-0 max-w-4xl px-6 pt-4 md:px-12">
      <div className="my-4">
        <Link
          className="hover:text-gray-1000 mb-16 flex flex-row items-center gap-2 text-sm font-normal text-gray-900 no-underline transition-all"
          href={getLocalizedPath(lang, "/blog")}
          prefetch
        >
          <ArrowLeft className="size-3" />
          Back to blog
        </Link>
      </div>

      <Mdx components={getMDXComponents({ isBlog: true })} />
    </article>
  );
}
