import Link from "next/link";
import { notFound } from "next/navigation";
import { ArrowLeftIcon } from "@heroicons/react/outline";
import type { Metadata } from "next";
import { blog } from "#app/source.ts";
import { createMetadata } from "#lib/create-metadata.ts";
import { FaviconHandler } from "#app/_components/favicon-handler.tsx";
import { mdxComponents } from "#mdx-components.tsx";

export function generateStaticParams(): Array<{ slug: Array<string> }> {
  return blog.getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export async function generateMetadata(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = blog.getPage(params.slug);

  if (!page) notFound();

  const version = params.slug?.[0] || "▲";

  const createOgUrl = () => {
    const groups = /^turbo-(?<major>\d+)-(?<minor>\d+)(?:-\d+)*$/.exec(version);
    if (groups) {
      const { major, minor } = groups.groups as {
        major: string;
        minor: string;
      };
      return `/api/og/blog?version=${encodeURIComponent(`${major}.${minor}`)}`;
    }

    return "▲";
  };

  return {
    ...createMetadata({
      title: page.data.title,
      description: page.data.description,
      canonicalPath: `/blog/${params.slug?.join("/") ?? ""}`,
    }),
    openGraph: {
      images: [
        {
          url: page.data.ogImage ?? createOgUrl(),
        },
      ],
    },
  };
}

export default async function Page(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<JSX.Element> {
  const params = await props.params;
  const page = blog.getPage(params.slug);

  if (!page) notFound();

  // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Not being inferred correctly
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

      {}
      <Mdx components={mdxComponents} />
    </article>
  );
}
