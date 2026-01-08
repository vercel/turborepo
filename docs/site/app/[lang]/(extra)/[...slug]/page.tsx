import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { extraPages } from "@/lib/geistdocs/source";

interface PageProps {
  params: Promise<{ slug?: string[]; lang: string }>;
}

const Page = async ({ params }: PageProps) => {
  const { slug } = await params;
  const page = extraPages.getPage(slug);

  if (!page) {
    notFound();
  }

  const MDX = page.data.body;

  return (
    <article className="prose mx-auto mb-10 w-full min-w-0 max-w-5xl px-6 pt-10 md:px-12">
      <h1 className="text-left">{page.data.title}</h1>
      <MDX components={getMDXComponents()} />
    </article>
  );
};

export function generateStaticParams(): Array<{ slug: string[] }> {
  return extraPages.getPages().map((page) => ({
    slug: page.slugs
  }));
}

export async function generateMetadata({
  params
}: PageProps): Promise<Metadata> {
  const { slug } = await params;
  const page = extraPages.getPage(slug);

  if (!page) {
    notFound();
  }

  const canonicalPath = slug?.join("/") ?? "";

  return {
    title: `${page.data.title} | Turborepo`,
    description: page.data.description,
    openGraph: {
      siteName: "Turborepo",
      url: `/${canonicalPath}`
    },
    alternates: {
      canonical: `/${canonicalPath}`
    }
  };
}

export default Page;
