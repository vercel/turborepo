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
    <article className="prose mx-auto mt-14 mb-10 w-full min-w-0 max-w-4xl px-6 pt-4 md:px-12">
      <FaviconHandler />
      <div className="my-4">
        <Link
          className="hover:text-foreground flex flex-row gap-2 text-sm text-gray-500  transition-all dark:text-gray-400"
          href="/blog"
        >
          <ArrowLeftIcon width=".75rem" />
          Back to blog
        </Link>
      </div>

      {/* TODO: Currently, the content is controlling the <h1 className="text-center"> to get the heading centered.
      /* Needs to be controlled here so we can just write the markdown and it will do the right thing.
       * */}
      <Mdx components={mdxComponents} />
    </article>
  );
}
