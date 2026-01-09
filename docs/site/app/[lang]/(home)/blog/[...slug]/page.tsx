import Link from "next/link";
import { notFound } from "next/navigation";
import { ArrowLeft } from "lucide-react";
import type { Metadata } from "next";
import { blog } from "@/lib/geistdocs/source";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { createSignedBlogOgUrl } from "@/lib/og/sign";

export function generateStaticParams(): Array<{ slug: Array<string> }> {
  return blog.getPages().map((page) => ({
    slug: page.slugs
  }));
}

export async function generateMetadata(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = blog.getPage(params.slug);

  if (!page) notFound();

  const version = params.slug?.[0] || "";

  const createOgUrl = () => {
    const groups = /^turbo-(?<major>\d+)-(?<minor>\d+)(?:-\d+)*$/.exec(version);
    if (groups) {
      const { major, minor } = groups.groups as {
        major: string;
        minor: string;
      };
      return createSignedBlogOgUrl(`${major}.${minor}`);
    }

    return "";
  };

  return {
    title: page.data.title,
    description: page.data.description,
    openGraph: {
      images:
        page.data.ogImage || createOgUrl()
          ? [
              {
                url: page.data.ogImage ?? createOgUrl()
              }
            ]
          : undefined
    }
  };
}

export default async function Page(props: {
  params: Promise<{ slug?: Array<string> }>;
}) {
  const params = await props.params;
  const page = blog.getPage(params.slug);

  if (!page) notFound();

  const Mdx = page.data.body;

  return (
    <article className="prose dark:prose-invert mx-auto mb-10 w-full min-w-0 max-w-4xl px-6 pt-4 md:px-12">
      <div className="my-4">
        <Link
          className="hover:text-foreground mb-16 flex flex-row gap-2 text-sm text-gray-600 dark:text-gray-400 no-underline transition-all"
          href="/blog"
        >
          <ArrowLeft className="w-3" />
          Back to blog
        </Link>
      </div>

      <Mdx components={getMDXComponents({ isBlog: true })} />
    </article>
  );
}
