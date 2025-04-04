import Link from "next/link";
import { notFound } from "next/navigation";
import { ArrowLeftIcon } from "@heroicons/react/outline";
import type { Metadata } from "next";
import { blog } from "@/app/source";
import { createMetadata } from "@/lib/create-metadata";
import { FaviconHandler } from "@/app/_components/favicon-handler";
import { mdxComponents } from "@/mdx-components";

export function generateStaticParams(): { slug: string[] }[] {
  return blog.getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export async function generateMetadata(props: {
  params: Promise<{ slug?: string[] }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = blog.getPage(params.slug);

  if (!page) notFound();

  return {
    ...createMetadata({
      title: page.data.title,
      description: page.data.description,
      canonicalPath: `/blog/${params.slug?.join("/") ?? ""}`,
    }),
    openGraph: {
      images: [
        {
          url: `/images/blog/${params.slug?.[0]}/x-card.png`,
        },
      ],
    },
  };
}

export default async function Page(props: {
  params: Promise<{ slug?: string[] }>;
}): Promise<JSX.Element> {
  const params = await props.params;
  const page = blog.getPage(params.slug);

  if (!page) notFound();

  const Mdx = page.data.body;

  return (
    <article className="prose mx-auto mb-10 w-full min-w-0 max-w-4xl px-6 pt-4 md:px-12">
      <FaviconHandler />
      <div className="my-4">
        <Link
          className="hover:text-foreground mb-16 flex flex-row gap-2 text-sm text-gray-900 no-underline transition-all dark:text-gray-900"
          href="/blog"
        >
          <ArrowLeftIcon width=".75rem" />
          Back to blog
        </Link>
      </div>

      <Mdx components={mdxComponents} />
    </article>
  );
}
